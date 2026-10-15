//! Second toolbar shown in advanced mode: tables, special characters, links,
//! case, date/time, statistics.

use std::cell::{OnceCell, RefCell};
use std::time::Duration;

use gtk::gio;
use gtk::glib;
use adw::prelude::*;
use gtk::subclass::prelude::*;

use crate::editor::{Editor, TextCase};
use crate::ui::special_chars::SpecialCharsPopover;
use crate::ui::stats_dialog::stats_dialog;

const PICKER_SIZE: usize = 8;

const MONTHS: [&str; 12] = [
    "janvier", "février", "mars", "avril", "mai", "juin", "juillet", "août", "septembre",
    "octobre", "novembre", "décembre",
];

/// "15 octobre 2026" (the day of month 1 is written "1er").
pub(crate) fn format_date_long(day: u32, month: u32, year: i32) -> String {
    let m = MONTHS[(month.clamp(1, 12) - 1) as usize];
    if day == 1 {
        format!("1er {m} {year}")
    } else {
        format!("{day} {m} {year}")
    }
}

pub(crate) fn format_date_short(day: u32, month: u32, year: i32) -> String {
    format!("{day:02}/{month:02}/{year}")
}

pub(crate) fn format_date_time(day: u32, month: u32, year: i32, hour: u32, min: u32) -> String {
    format!("{} {hour:02}:{min:02}", format_date_short(day, month, year))
}

/// Adds "https://" when the address has no scheme; empty input gives `None`.
pub(crate) fn normalize_url(input: &str) -> Option<String> {
    let s = input.trim();
    if s.is_empty() {
        return None;
    }
    let has_scheme = s
        .split_once(':')
        .map(|(scheme, rest)| {
            !scheme.is_empty()
                && scheme.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
                && scheme
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
                // "example.com:8080" is a host and port, not a scheme.
                && (scheme.eq_ignore_ascii_case("mailto")
                    || scheme.eq_ignore_ascii_case("tel")
                    || rest.starts_with("//"))
        })
        .unwrap_or(false);
    Some(if has_scheme { s.to_string() } else { format!("https://{s}") })
}

fn looks_like_url(s: &str) -> bool {
    let s = s.trim();
    !s.is_empty()
        && !s.contains(char::is_whitespace)
        && (s.contains("://") || s.starts_with("www.") || s.starts_with("mailto:"))
}

pub struct Controls {
    table_ops: gtk::MenuButton,
    link_button: gtk::MenuButton,
    link_popover: gtk::Popover,
    link_entry: gtk::Entry,
    link_remove: gtk::Button,
    chars_button: gtk::MenuButton,
    picker_label: gtk::Label,
    picker_cells: Vec<gtk::Widget>,
    stats: gtk::Button,
    others: Vec<gtk::Widget>,
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct AdvancedBar {
        pub controls: OnceCell<Controls>,
        pub editor: glib::WeakRef<Editor>,
        pub handler: RefCell<Option<glib::SignalHandlerId>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for AdvancedBar {
        const NAME: &'static str = "GwAdvancedBar";
        type Type = super::AdvancedBar;
        type ParentType = gtk::Box;
    }

    impl ObjectImpl for AdvancedBar {}
    impl WidgetImpl for AdvancedBar {}
    impl BoxImpl for AdvancedBar {}
}

glib::wrapper! {
    pub struct AdvancedBar(ObjectSubclass<imp::AdvancedBar>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl Default for AdvancedBar {
    fn default() -> Self {
        Self::new()
    }
}

fn menu_button(icon: Option<&str>, label: Option<&str>, tip: &str) -> gtk::MenuButton {
    let b = gtk::MenuButton::new();
    if let Some(i) = icon {
        b.set_icon_name(i);
    }
    if let Some(l) = label {
        b.set_label(l);
    }
    b.set_tooltip_text(Some(tip));
    b.add_css_class("flat");
    b
}

fn pop<W: IsA<gtk::Widget>>(w: &W) {
    w.add_css_class("pop");
    let weak = w.downgrade();
    glib::timeout_add_local_once(Duration::from_millis(200), move || {
        if let Some(w) = weak.upgrade() {
            w.remove_css_class("pop");
        }
    });
}

fn separator() -> gtk::Separator {
    let s = gtk::Separator::new(gtk::Orientation::Vertical);
    s.add_css_class("spacer");
    s
}

fn menu(items: &[(&str, &str)]) -> gio::Menu {
    let m = gio::Menu::new();
    for (label, action) in items {
        m.append(Some(label), Some(action));
    }
    m
}

impl AdvancedBar {
    pub fn new() -> Self {
        let bar: Self = glib::Object::new();
        bar.build();
        bar.set_sensitive(false);
        bar
    }

    fn with_editor(&self, f: impl FnOnce(&Editor)) {
        if let Some(ed) = self.imp().editor.upgrade() {
            f(&ed);
        }
    }

    fn build(&self) {
        self.add_css_class("toolbar");
        self.add_css_class("format-bar");
        self.add_css_class("advanced-bar");
        self.set_spacing(6);

        // Centered like the format bar's content: both rows read as one toolbar.
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        content.set_halign(gtk::Align::Center);
        content.set_hexpand(true);
        content.set_margin_start(6);
        content.set_margin_end(6);
        let mut others: Vec<gtk::Widget> = Vec::new();

        // --- Table picker -------------------------------------------------
        let table_button = menu_button(Some("view-grid-symbolic"), None, "Insérer un tableau");
        let picker_label = gtk::Label::new(Some("Choisissez la taille"));
        picker_label.add_css_class("dim-label");
        let grid = gtk::Grid::builder().row_spacing(3).column_spacing(3).build();
        let mut cells: Vec<gtk::Widget> = Vec::new();
        for r in 0..PICKER_SIZE {
            for c in 0..PICKER_SIZE {
                let cell = gtk::Box::new(gtk::Orientation::Horizontal, 0);
                cell.add_css_class("table-picker-cell");
                cell.set_size_request(18, 18);
                let motion = gtk::EventControllerMotion::new();
                motion.connect_enter(glib::clone!(
                    #[weak(rename_to = bar)]
                    self,
                    move |_, _, _| bar.highlight(r + 1, c + 1)
                ));
                cell.add_controller(motion);
                let click = gtk::GestureClick::new();
                click.connect_released(glib::clone!(
                    #[weak(rename_to = bar)]
                    self,
                    #[weak]
                    table_button,
                    move |_, _, _, _| {
                        table_button.popdown();
                        bar.with_editor(|ed| ed.insert_table(r + 1, c + 1));
                    }
                ));
                cell.add_controller(click);
                grid.attach(&cell, c as i32, r as i32, 1, 1);
                cells.push(cell.upcast());
            }
        }
        let picker = gtk::Box::new(gtk::Orientation::Vertical, 8);
        picker.set_margin_top(8);
        picker.set_margin_bottom(8);
        picker.set_margin_start(8);
        picker.set_margin_end(8);
        picker.append(&grid);
        picker.append(&picker_label);
        let popover = gtk::Popover::new();
        popover.set_child(Some(&picker));
        popover.connect_show(glib::clone!(
            #[weak(rename_to = bar)]
            self,
            move |_| bar.reset_picker()
        ));
        table_button.set_popover(Some(&popover));
        others.push(table_button.clone().upcast());

        // --- Table operations ---------------------------------------------
        let table_ops = menu_button(
            Some("view-more-horizontal-symbolic"),
            None,
            "Opérations sur le tableau",
        );
        let ops = gio::Menu::new();
        ops.append_section(
            None,
            &menu(&[
                ("Insérer une ligne au-dessus", "adv.row-above"),
                ("Insérer une ligne en dessous", "adv.row-below"),
                ("Insérer une colonne à gauche", "adv.col-left"),
                ("Insérer une colonne à droite", "adv.col-right"),
            ]),
        );
        ops.append_section(
            None,
            &menu(&[
                ("Supprimer la ligne", "adv.row-delete"),
                ("Supprimer la colonne", "adv.col-delete"),
                ("Supprimer le tableau", "adv.table-delete"),
            ]),
        );
        table_ops.set_menu_model(Some(&ops));
        let tables = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        tables.add_css_class("linked");
        tables.append(&table_button);
        tables.append(&table_ops);
        content.append(&tables);
        content.append(&separator());

        // --- Special characters and emoji ---------------------------------
        let chars_button = menu_button(
            Some("accessories-character-map-symbolic"),
            None,
            "Caractères spéciaux",
        );
        let chars = SpecialCharsPopover::new();
        chars.connect_char_selected(glib::clone!(
            #[weak(rename_to = bar)]
            self,
            move |ch| bar.with_editor(|ed| ed.insert_text_at_cursor(ch))
        ));
        chars_button.set_popover(Some(&chars));
        let emoji = gtk::Button::from_icon_name("face-smile-symbolic");
        emoji.set_tooltip_text(Some("Insérer un émoji"));
        emoji.add_css_class("flat");
        emoji.connect_clicked(glib::clone!(
            #[weak(rename_to = bar)]
            self,
            move |b| {
                pop(b);
                bar.with_editor(|ed| {
                    let active = ed.active_editor();
                    active.grab_focus();
                    active.emit_insert_emoji();
                });
            }
        ));
        let specials = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        specials.add_css_class("linked");
        specials.append(&chars_button);
        specials.append(&emoji);
        content.append(&specials);
        others.push(chars_button.clone().upcast());
        others.push(emoji.clone().upcast());

        // --- Link ----------------------------------------------------------
        let link_button = menu_button(
            Some("insert-link-symbolic"),
            None,
            "Insérer un lien (Ctrl+K)",
        );
        let link_entry = gtk::Entry::builder()
            .placeholder_text("Adresse (https://…)")
            .width_chars(32)
            .activates_default(false)
            .build();
        let apply = gtk::Button::with_label("Appliquer");
        apply.add_css_class("suggested-action");
        let link_remove = gtk::Button::with_label("Retirer le lien");
        link_remove.add_css_class("destructive-action");
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        actions.set_halign(gtk::Align::End);
        actions.append(&link_remove);
        actions.append(&apply);
        let lbox = gtk::Box::new(gtk::Orientation::Vertical, 8);
        lbox.set_margin_top(10);
        lbox.set_margin_bottom(10);
        lbox.set_margin_start(10);
        lbox.set_margin_end(10);
        lbox.append(&link_entry);
        lbox.append(&actions);
        let link_popover = gtk::Popover::new();
        link_popover.set_child(Some(&lbox));
        link_popover.connect_show(glib::clone!(
            #[weak(rename_to = bar)]
            self,
            move |_| bar.fill_link_popover()
        ));
        link_button.set_popover(Some(&link_popover));
        let on_apply = glib::clone!(
            #[weak(rename_to = bar)]
            self,
            move || bar.apply_link()
        );
        apply.connect_clicked({
            let f = on_apply.clone();
            move |_| f()
        });
        link_entry.connect_activate(move |_| on_apply());
        link_remove.connect_clicked(glib::clone!(
            #[weak(rename_to = bar)]
            self,
            move |_| {
                bar.with_editor(|ed| ed.set_link(None));
                bar.imp().controls.get().unwrap().link_button.popdown();
            }
        ));
        content.append(&link_button);
        others.push(link_button.clone().upcast());

        // --- Case ----------------------------------------------------------
        let case_button = menu_button(None, Some("Aa"), "Changer la casse");
        case_button.set_menu_model(Some(&menu(&[
            ("MAJUSCULES", "adv.case-upper"),
            ("minuscules", "adv.case-lower"),
            ("Majuscule En Début De Mot", "adv.case-title"),
            ("Majuscule en début de phrase", "adv.case-sentence"),
        ])));
        content.append(&case_button);
        others.push(case_button.clone().upcast());

        // --- Date and time -------------------------------------------------
        let date_button = menu_button(
            Some("x-office-calendar-symbolic"),
            None,
            "Insérer la date et l'heure",
        );
        let now = glib::DateTime::now_local().ok();
        let (long, short, full) = match &now {
            Some(n) => {
                let (d, m, y) = (n.day_of_month() as u32, n.month() as u32, n.year());
                (
                    format_date_long(d, m, y),
                    format_date_short(d, m, y),
                    format_date_time(d, m, y, n.hour() as u32, n.minute() as u32),
                )
            }
            None => Default::default(),
        };
        date_button.set_menu_model(Some(&menu(&[
            (&long, "adv.date-long"),
            (&short, "adv.date-short"),
            (&full, "adv.date-time"),
        ])));
        // Labels are refreshed after the menu CLOSES, never while it is shown:
        // replacing the model destroys the popover, which would close it while
        // opening, or drop the action of the item just clicked (the popover
        // closes before the action is activated). Hence the delay.
        date_button.connect_notify_local(Some("active"), |b, _| {
            if b.is_active() {
                return;
            }
            let weak = b.downgrade();
            glib::timeout_add_local_once(std::time::Duration::from_millis(400), move || {
                let Some(b) = weak.upgrade() else { return };
                if b.is_active() {
                    return;
                }
                if let Ok(n) = glib::DateTime::now_local() {
                    let (d, m, y) = (n.day_of_month() as u32, n.month() as u32, n.year());
                    b.set_menu_model(Some(&menu(&[
                        (&format_date_long(d, m, y), "adv.date-long"),
                        (&format_date_short(d, m, y), "adv.date-short"),
                        (
                            &format_date_time(d, m, y, n.hour() as u32, n.minute() as u32),
                            "adv.date-time",
                        ),
                    ])));
                }
            });
        });
        content.append(&date_button);
        others.push(date_button.clone().upcast());
        content.append(&separator());

        // --- Statistics ----------------------------------------------------
        let stats = gtk::Button::from_icon_name("document-properties-symbolic");
        stats.set_tooltip_text(Some("Statistiques du document"));
        stats.add_css_class("flat");
        stats.connect_clicked(glib::clone!(
            #[weak(rename_to = bar)]
            self,
            move |b| {
                pop(b);
                bar.show_statistics();
            }
        ));
        content.append(&stats);
        others.push(stats.clone().upcast());

        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::External)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .hexpand(true)
            .child(&content)
            .build();
        self.append(&scroller);

        self.install_actions();

        let _ = self.imp().controls.set(Controls {
            table_ops,
            link_button,
            link_popover,
            link_entry,
            link_remove,
            chars_button,
            picker_label,
            picker_cells: cells,
            stats,
            others,
        });
    }

    fn install_actions(&self) {
        let group = gio::SimpleActionGroup::new();
        let add = |name: &str, f: Box<dyn Fn(&Editor)>| {
            let a = gio::SimpleAction::new(name, None);
            a.connect_activate(glib::clone!(
                #[weak(rename_to = bar)]
                self,
                move |_, _| bar.with_editor(|ed| f(ed))
            ));
            group.add_action(&a);
        };
        add("row-above", Box::new(|e| e.table_insert_row(false)));
        add("row-below", Box::new(|e| e.table_insert_row(true)));
        add("col-left", Box::new(|e| e.table_insert_column(false)));
        add("col-right", Box::new(|e| e.table_insert_column(true)));
        add("row-delete", Box::new(|e| e.table_remove_row()));
        add("col-delete", Box::new(|e| e.table_remove_column()));
        add("table-delete", Box::new(|e| e.table_delete()));
        add("case-upper", Box::new(|e| e.change_case(TextCase::Upper)));
        add("case-lower", Box::new(|e| e.change_case(TextCase::Lower)));
        add("case-title", Box::new(|e| e.change_case(TextCase::Title)));
        add("case-sentence", Box::new(|e| e.change_case(TextCase::Sentence)));
        let date = |f: fn(&glib::DateTime) -> String| -> Box<dyn Fn(&Editor)> {
            Box::new(move |e| {
                if let Ok(n) = glib::DateTime::now_local() {
                    e.insert_text_at_cursor(&f(&n));
                }
            })
        };
        add(
            "date-long",
            date(|n| format_date_long(n.day_of_month() as u32, n.month() as u32, n.year())),
        );
        add(
            "date-short",
            date(|n| format_date_short(n.day_of_month() as u32, n.month() as u32, n.year())),
        );
        add(
            "date-time",
            date(|n| {
                format_date_time(
                    n.day_of_month() as u32,
                    n.month() as u32,
                    n.year(),
                    n.hour() as u32,
                    n.minute() as u32,
                )
            }),
        );
        self.insert_action_group("adv", Some(&group));
    }

    fn highlight(&self, rows: usize, cols: usize) {
        let Some(c) = self.imp().controls.get() else { return };
        for (i, cell) in c.picker_cells.iter().enumerate() {
            if i / PICKER_SIZE < rows && i % PICKER_SIZE < cols {
                cell.add_css_class("selected");
            } else {
                cell.remove_css_class("selected");
            }
        }
        c.picker_label.set_label(&format!("{rows} × {cols}"));
    }

    fn reset_picker(&self) {
        let Some(c) = self.imp().controls.get() else { return };
        for cell in &c.picker_cells {
            cell.remove_css_class("selected");
        }
        c.picker_label.set_label("Choisissez la taille");
    }

    fn fill_link_popover(&self) {
        let Some(c) = self.imp().controls.get() else { return };
        let mut text = String::new();
        let mut has_link = false;
        self.with_editor(|ed| {
            let st = ed.format_state();
            has_link = st.link.is_some();
            text = match st.link {
                Some(l) => l,
                None => {
                    let sel = ed.selected_text();
                    if looks_like_url(&sel) { sel.trim().to_string() } else { String::new() }
                }
            };
        });
        c.link_entry.set_text(&text);
        c.link_remove.set_visible(has_link);
        c.link_entry.grab_focus();
        c.link_entry.set_position(-1);
    }

    fn apply_link(&self) {
        let Some(c) = self.imp().controls.get() else { return };
        let Some(url) = normalize_url(&c.link_entry.text()) else {
            c.link_entry.add_css_class("error");
            let weak = c.link_entry.downgrade();
            glib::timeout_add_local_once(Duration::from_millis(600), move || {
                if let Some(e) = weak.upgrade() {
                    e.remove_css_class("error");
                }
            });
            return;
        };
        c.link_popover.popdown();
        self.with_editor(|ed| {
            if ed.selected_text().is_empty() && ed.format_state().link.is_none() {
                // Nothing to link: insert the address itself, select it, link it.
                ed.insert_text_at_cursor(&url);
                let buffer = ed.active_editor().buffer();
                let end = buffer.iter_at_mark(&buffer.get_insert());
                let mut start = end;
                start.backward_chars(url.chars().count() as i32);
                buffer.select_range(&end, &start);
            }
            ed.set_link(Some(&url));
        });
    }

    /// Binds to the editor of the active tab; disconnects the previous one.
    pub fn set_editor(&self, editor: Option<&Editor>) {
        let imp = self.imp();
        if let Some(id) = imp.handler.take() {
            if let Some(old) = imp.editor.upgrade() {
                old.disconnect(id);
            }
        }
        imp.editor.set(editor);
        match editor {
            Some(ed) => {
                let weak = self.downgrade();
                let id = ed.connect_format_changed(move |ed| {
                    if let Some(bar) = weak.upgrade() {
                        bar.sync(ed);
                    }
                });
                imp.handler.replace(Some(id));
                self.set_sensitive(true);
                self.sync(ed);
            }
            None => self.set_sensitive(false),
        }
    }

    fn sync(&self, ed: &Editor) {
        let Some(c) = self.imp().controls.get() else { return };
        let st = ed.format_state();
        c.table_ops.set_sensitive(st.in_table);
        if st.link.is_some() {
            c.link_button.add_css_class("has-link");
        } else {
            c.link_button.remove_css_class("has-link");
        }
        let _ = (&c.others, &c.stats, &c.chars_button);
    }

    /// Opens the link popover (Ctrl+K), pre-filled with the link at the cursor.
    pub fn popup_link_editor(&self) {
        if let Some(c) = self.imp().controls.get() {
            c.link_button.popup();
        }
    }

    /// Opens the special characters popover.
    pub fn popup_special_chars(&self) {
        if let Some(c) = self.imp().controls.get() {
            c.chars_button.popup();
        }
    }

    /// Opens the statistics dialog for the bound editor.
    pub fn show_statistics(&self) {
        self.with_editor(|ed| {
            let dialog = stats_dialog(&ed.statistics());
            dialog.present(self.root().as_ref());
        });
    }
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn dates() {
        assert_eq!(format_date_long(15, 10, 2026), "15 octobre 2026");
        assert_eq!(format_date_long(1, 8, 2026), "1er août 2026");
        assert_eq!(format_date_short(5, 3, 2026), "05/03/2026");
        assert_eq!(format_date_time(15, 10, 2026, 13, 5), "15/10/2026 13:05");
    }

    #[test]
    fn urls() {
        assert_eq!(normalize_url("  "), None);
        assert_eq!(normalize_url("example.com").as_deref(), Some("https://example.com"));
        assert_eq!(normalize_url("http://a.b").as_deref(), Some("http://a.b"));
        assert_eq!(normalize_url("mailto:a@b.c").as_deref(), Some("mailto:a@b.c"));
        assert_eq!(normalize_url("localhost:8080").as_deref(), Some("https://localhost:8080"));
    }
}

/// GTK-dependent checks, to be called from `crate::gtk_tests`.
#[cfg(test)]
pub(crate) mod tests {
    pub(crate) fn bar_builds() {
        let bar = super::AdvancedBar::new();
        bar.set_editor(None);
        assert!(!gtk::prelude::WidgetExt::is_sensitive(&bar));
    }
}
