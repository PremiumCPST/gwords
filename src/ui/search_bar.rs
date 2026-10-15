//! Find & replace bar.

use std::cell::{Cell, OnceCell, RefCell};
use std::time::Duration;

use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use crate::editor::tags;
use crate::editor::Editor;

/// Delay between the last keystroke and the (re)search.
const DEBOUNCE: Duration = Duration::from_millis(150);
/// Upper bound on the number of highlighted matches.
const MAX_MATCHES: usize = 10_000;

/// Widgets of the bar, built once in `constructed`.
#[derive(Clone)]
struct Widgets {
    revealer: gtk::Revealer,
    entry: gtk::SearchEntry,
    label: gtk::Label,
    case_toggle: gtk::ToggleButton,
    replace_toggle: gtk::ToggleButton,
    replace_revealer: gtk::Revealer,
    replace_entry: gtk::Entry,
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct SearchBar {
        pub(super) widgets: OnceCell<Widgets>,
        /// Editor currently searched (weak: the tab may be closed).
        pub(super) editor: RefCell<Option<glib::WeakRef<Editor>>>,
        /// Buffer "changed" handler on the current editor's buffer.
        pub(super) buffer_handler: RefCell<Option<(glib::WeakRef<gtk::TextBuffer>, glib::SignalHandlerId)>>,
        /// Pending debounced search, with its `jump` flag.
        pub(super) debounce: RefCell<Option<(glib::SourceId, bool)>>,
        /// Match ranges as (start, end) character offsets, in buffer order.
        pub(super) matches: RefCell<Vec<(i32, i32)>>,
        pub(super) capped: Cell<bool>,
        pub(super) current: Cell<Option<usize>>,
        pub(super) visible: Cell<bool>,
        /// Transient status shown instead of the match counter (e.g. "12 remplacements").
        pub(super) message: RefCell<Option<String>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SearchBar {
        const NAME: &'static str = "GwSearchBar";
        type Type = super::SearchBar;
        type ParentType = gtk::Box;
    }

    impl ObjectImpl for SearchBar {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            obj.set_orientation(gtk::Orientation::Vertical);

            let revealer = gtk::Revealer::builder()
                .transition_type(gtk::RevealerTransitionType::SlideDown)
                .transition_duration(200)
                .build();

            let bar = gtk::Box::new(gtk::Orientation::Vertical, 6);
            bar.add_css_class("search-bar");
            bar.add_css_class("toolbar");
            bar.set_margin_start(12);
            bar.set_margin_end(12);
            bar.set_margin_top(6);
            bar.set_margin_bottom(6);

            // Row 1: search.
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            let entry = gtk::SearchEntry::new();
            entry.set_placeholder_text(Some("Rechercher"));
            entry.set_hexpand(true);

            let label = gtk::Label::new(None);
            label.add_css_class("dim-label");
            label.add_css_class("numeric");

            let nav = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            nav.add_css_class("linked");
            let prev = gtk::Button::from_icon_name("go-up-symbolic");
            prev.set_tooltip_text(Some("Précédent (Maj+Entrée)"));
            let next = gtk::Button::from_icon_name("go-down-symbolic");
            next.set_tooltip_text(Some("Suivant (Entrée)"));
            nav.append(&prev);
            nav.append(&next);

            let case_toggle = gtk::ToggleButton::new();
            case_toggle.set_label("Aa");
            case_toggle.set_tooltip_text(Some("Respecter la casse"));

            let replace_toggle = gtk::ToggleButton::builder()
                .icon_name("edit-find-replace-symbolic")
                .tooltip_text("Afficher le remplacement")
                .build();

            let close = gtk::Button::from_icon_name("window-close-symbolic");
            close.add_css_class("flat");
            close.add_css_class("circular");
            close.set_tooltip_text(Some("Fermer"));

            row.append(&entry);
            row.append(&label);
            row.append(&nav);
            row.append(&case_toggle);
            row.append(&replace_toggle);
            row.append(&close);

            // Row 2: replace, in its own revealer.
            let replace_revealer = gtk::Revealer::builder()
                .transition_type(gtk::RevealerTransitionType::SlideDown)
                .transition_duration(200)
                .build();
            let row2 = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            let replace_entry = gtk::Entry::new();
            replace_entry.set_placeholder_text(Some("Remplacer par"));
            replace_entry.set_hexpand(true);
            let btn_replace = gtk::Button::with_label("Remplacer");
            let btn_replace_all = gtk::Button::with_label("Tout remplacer");
            row2.append(&replace_entry);
            row2.append(&btn_replace);
            row2.append(&btn_replace_all);
            replace_revealer.set_child(Some(&row2));

            bar.append(&row);
            bar.append(&replace_revealer);
            revealer.set_child(Some(&bar));
            obj.append(&revealer);

            // Signal handlers: all capture weak references to the bar.
            entry.connect_search_changed(glib::clone!(
                #[weak]
                obj,
                move |_| {
                    obj.imp().message.take();
                    obj.schedule_search(true);
                }
            ));
            entry.connect_activate(glib::clone!(
                #[weak]
                obj,
                move |_| obj.step(true)
            ));
            entry.connect_next_match(glib::clone!(
                #[weak]
                obj,
                move |_| obj.step(true)
            ));
            entry.connect_previous_match(glib::clone!(
                #[weak]
                obj,
                move |_| obj.step(false)
            ));
            entry.connect_stop_search(glib::clone!(
                #[weak]
                obj,
                move |_| obj.hide()
            ));

            // Shift+Entrée goes to the previous match (handled before the entry's activate).
            let keys = gtk::EventControllerKey::new();
            keys.set_propagation_phase(gtk::PropagationPhase::Capture);
            keys.connect_key_pressed(glib::clone!(
                #[weak]
                obj,
                #[upgrade_or]
                glib::Propagation::Proceed,
                move |_, key, _, state| {
                    if matches!(key, gdk::Key::Return | gdk::Key::KP_Enter)
                        && state.contains(gdk::ModifierType::SHIFT_MASK)
                    {
                        obj.step(false);
                        glib::Propagation::Stop
                    } else {
                        glib::Propagation::Proceed
                    }
                }
            ));
            entry.add_controller(keys);

            case_toggle.connect_toggled(glib::clone!(
                #[weak]
                obj,
                move |_| obj.schedule_search(true)
            ));
            prev.connect_clicked(glib::clone!(
                #[weak]
                obj,
                move |_| obj.step(false)
            ));
            next.connect_clicked(glib::clone!(
                #[weak]
                obj,
                move |_| obj.step(true)
            ));
            replace_toggle.connect_toggled(glib::clone!(
                #[weak]
                replace_revealer,
                move |t| replace_revealer.set_reveal_child(t.is_active())
            ));
            close.connect_clicked(glib::clone!(
                #[weak]
                obj,
                move |_| obj.hide()
            ));
            btn_replace.connect_clicked(glib::clone!(
                #[weak]
                obj,
                move |_| obj.replace_current()
            ));
            btn_replace_all.connect_clicked(glib::clone!(
                #[weak]
                obj,
                move |_| obj.replace_all()
            ));

            let esc = gtk::EventControllerKey::new();
            esc.connect_key_pressed(glib::clone!(
                #[weak]
                obj,
                #[upgrade_or]
                glib::Propagation::Proceed,
                move |_, key, _, _| {
                    if key == gdk::Key::Escape {
                        obj.hide();
                        glib::Propagation::Stop
                    } else {
                        glib::Propagation::Proceed
                    }
                }
            ));
            replace_entry.add_controller(esc);

            btn_replace.set_tooltip_text(Some("Remplacer l'occurrence courante"));
            btn_replace_all.set_tooltip_text(Some("Remplacer toutes les occurrences"));

            let _ = self.widgets.set(Widgets {
                revealer,
                entry,
                label,
                case_toggle,
                replace_toggle,
                replace_revealer,
                replace_entry,
            });
        }
    }

    impl WidgetImpl for SearchBar {}
    impl BoxImpl for SearchBar {}
}

glib::wrapper! {
    pub struct SearchBar(ObjectSubclass<imp::SearchBar>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl Default for SearchBar {
    fn default() -> Self {
        Self::new()
    }
}

/// Removes the transient search highlights from the whole buffer.
fn clear_tags(buffer: &gtk::TextBuffer) {
    let start = buffer.start_iter();
    let end = buffer.end_iter();
    buffer.remove_tag_by_name(tags::SEARCH_MATCH, &start, &end);
    buffer.remove_tag_by_name(tags::SEARCH_CURRENT, &start, &end);
}

impl SearchBar {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Binds to the editor of the active tab (`None` when no tab is open).
    /// Must disconnect handlers from the previous editor.
    pub fn set_editor(&self, editor: Option<&Editor>) {
        self.unbind_editor();
        let Some(editor) = editor else {
            self.update_label();
            return;
        };

        let buffer = editor.buffer();
        let obj = self.clone();
        let handler = buffer.connect_changed(glib::clone!(
            #[weak]
            obj,
            move |_| {
                if obj.imp().visible.get() {
                    obj.schedule_search(false);
                }
            }
        ));

        let imp = self.imp();
        *imp.editor.borrow_mut() = Some(editor.downgrade());
        *imp.buffer_handler.borrow_mut() = Some((buffer.downgrade(), handler));
        if imp.visible.get() {
            self.schedule_search(true);
        }
        self.update_label();
    }

    /// Disconnects from the current editor and clears its highlights.
    fn unbind_editor(&self) {
        let imp = self.imp();
        if let Some((weak, id)) = imp.buffer_handler.take() {
            if let Some(buffer) = weak.upgrade() {
                buffer.disconnect(id);
                clear_tags(&buffer);
            }
        }
        imp.editor.take();
        imp.matches.borrow_mut().clear();
        imp.capped.set(false);
        imp.current.set(None);
    }

    fn widgets(&self) -> &Widgets {
        self.imp().widgets.get().expect("search bar widgets are built in constructed")
    }

    /// The bound editor and its buffer, if both are still alive.
    fn active(&self) -> Option<(Editor, gtk::TextBuffer)> {
        let editor = self.imp().editor.borrow().as_ref()?.upgrade()?;
        let buffer = editor.buffer();
        Some((editor, buffer))
    }

    /// Reveals the bar (animated) and focuses the search entry, pre-filled
    /// with the current selection if any. `replace` also shows the replace row.
    pub fn show(&self, replace: bool) {
        let imp = self.imp();
        let w = self.widgets();
        imp.visible.set(true);
        imp.message.take();
        w.revealer.set_reveal_child(true);
        if replace {
            w.replace_toggle.set_active(true);
            w.replace_revealer.set_reveal_child(true);
        }

        if let Some((_, buffer)) = self.active() {
            if let Some((s, e)) = buffer.selection_bounds() {
                if s.line() == e.line() {
                    let text = buffer.text(&s, &e, false);
                    if !text.is_empty() {
                        w.entry.set_text(&text);
                    }
                }
            }
        }

        w.entry.grab_focus();
        w.entry.select_region(0, -1);
        self.schedule_search(true);
        self.update_label();
    }

    /// Hides the bar and clears search highlights.
    pub fn hide(&self) {
        let imp = self.imp();
        imp.visible.set(false);
        imp.message.take();
        if let Some((id, _)) = imp.debounce.take() {
            id.remove();
        }
        self.widgets().revealer.set_reveal_child(false);
        imp.matches.borrow_mut().clear();
        imp.capped.set(false);
        imp.current.set(None);
        if let Some((editor, buffer)) = self.active() {
            clear_tags(&buffer);
            editor.grab_focus();
        }
        self.update_label();
    }

    /// Schedules a search after `DEBOUNCE`. `jump` moves the selection to the
    /// first match after the cursor (otherwise only highlights are refreshed).
    fn schedule_search(&self, jump: bool) {
        let imp = self.imp();
        let jump = jump || imp.debounce.take().map(|(id, j)| {
            id.remove();
            j
        }).unwrap_or(false);

        let weak = self.downgrade();
        let id = glib::timeout_add_local(DEBOUNCE, move || {
            if let Some(obj) = weak.upgrade() {
                obj.imp().debounce.take();
                obj.run_search(jump);
            }
            glib::ControlFlow::Break
        });
        *imp.debounce.borrow_mut() = Some((id, jump));
    }

    /// Recomputes all matches and their highlights. With `jump`, the first
    /// match after the cursor becomes current and is selected and scrolled to.
    fn run_search(&self, jump: bool) {
        let imp = self.imp();
        let w = self.widgets();
        let Some((editor, buffer)) = self.active() else {
            self.update_label();
            return;
        };

        clear_tags(&buffer);
        imp.matches.borrow_mut().clear();
        imp.capped.set(false);

        let needle = w.entry.text();
        if !imp.visible.get() || needle.is_empty() {
            imp.current.set(None);
            self.update_label();
            return;
        }

        let mut flags = gtk::TextSearchFlags::TEXT_ONLY | gtk::TextSearchFlags::VISIBLE_ONLY;
        if !w.case_toggle.is_active() {
            flags |= gtk::TextSearchFlags::CASE_INSENSITIVE;
        }

        let mut found = Vec::new();
        let mut capped = false;
        let mut iter = buffer.start_iter();
        while let Some((s, e)) = iter.forward_search(needle.as_str(), flags, None) {
            if found.len() >= MAX_MATCHES {
                capped = true;
                break;
            }
            found.push((s.offset(), e.offset()));
            iter = e;
        }

        for &(s, e) in &found {
            buffer.apply_tag_by_name(tags::SEARCH_MATCH, &buffer.iter_at_offset(s), &buffer.iter_at_offset(e));
        }

        let current = if found.is_empty() {
            None
        } else if jump {
            let cursor = buffer.cursor_position();
            Some(found.iter().position(|&(s, _)| s >= cursor).unwrap_or(0))
        } else {
            imp.current.get().filter(|&i| i < found.len())
        };

        *imp.matches.borrow_mut() = found;
        imp.capped.set(capped);
        imp.current.set(current);
        if let Some(i) = current {
            self.highlight(&editor, &buffer, i, jump);
        }
        self.update_label();
    }

    /// Marks match `i` as current and, if `select`, selects and scrolls to it.
    fn highlight(&self, editor: &Editor, buffer: &gtk::TextBuffer, i: usize, select: bool) {
        let Some((s, e)) = self.imp().matches.borrow().get(i).copied() else {
            return;
        };
        buffer.remove_tag_by_name(tags::SEARCH_CURRENT, &buffer.start_iter(), &buffer.end_iter());
        let mut start = buffer.iter_at_offset(s);
        let end = buffer.iter_at_offset(e);
        buffer.apply_tag_by_name(tags::SEARCH_CURRENT, &start, &end);
        if select {
            buffer.select_range(&start, &end);
            editor.scroll_to_iter(&mut start, 0.2, false, 0.0, 0.0);
        }
    }

    /// Moves to the next (or previous) match, wrapping around.
    fn step(&self, forward: bool) {
        let imp = self.imp();
        let n = imp.matches.borrow().len();
        let cur = imp.current.get().filter(|&i| i < n);
        let Some(i) = cur else {
            // Nothing current yet (or no match): search from the cursor.
            self.run_search(true);
            return;
        };
        let next = if forward { (i + 1) % n } else { (i + n - 1) % n };
        imp.current.set(Some(next));
        imp.message.take();
        if let Some((editor, buffer)) = self.active() {
            self.highlight(&editor, &buffer, next, true);
        }
        self.update_label();
    }

    /// Replaces the current match, then moves to the next one.
    fn replace_current(&self) {
        let imp = self.imp();
        let replacement = self.widgets().replace_entry.text();
        // Refresh the matches first, in case the buffer changed since the last search.
        self.run_search(false);
        let Some((_, buffer)) = self.active() else {
            return;
        };

        let target = {
            let matches = imp.matches.borrow();
            let selection = buffer.selection_bounds().map(|(s, e)| (s.offset(), e.offset()));
            selection
                .and_then(|sel| matches.iter().position(|&m| m == sel))
                .or(imp.current.get().filter(|&i| i < matches.len()))
        };
        let Some(i) = target else {
            self.run_search(true);
            return;
        };
        let (s, e) = imp.matches.borrow()[i];

        buffer.begin_user_action();
        self.replace_range(&buffer, s, e, &replacement);
        buffer.end_user_action();

        self.run_search(true);
    }

    /// Replaces every match (from the end of the buffer to the start) in one undo step.
    fn replace_all(&self) {
        let imp = self.imp();
        let replacement = self.widgets().replace_entry.text();
        self.run_search(false);
        let matches = imp.matches.borrow().clone();
        let Some((_, buffer)) = self.active() else {
            return;
        };
        if matches.is_empty() {
            self.run_search(true);
            return;
        }

        buffer.begin_user_action();
        for &(s, e) in matches.iter().rev() {
            self.replace_range(&buffer, s, e, &replacement);
        }
        buffer.end_user_action();

        let count = matches.len();
        self.run_search(false);
        let unit = if count == 1 { "remplacement" } else { "remplacements" };
        *imp.message.borrow_mut() = Some(format!("{count} {unit}"));
        self.update_label();
    }

    /// Replaces the characters `[s, e)` by `replacement`, leaving the cursor after it.
    fn replace_range(&self, buffer: &gtk::TextBuffer, s: i32, e: i32, replacement: &str) {
        let mut start = buffer.iter_at_offset(s);
        let mut end = buffer.iter_at_offset(e);
        buffer.delete(&mut start, &mut end);
        if !replacement.is_empty() {
            let mut at = buffer.iter_at_offset(s);
            buffer.insert(&mut at, replacement);
        }
        let caret = s + replacement.chars().count() as i32;
        buffer.place_cursor(&buffer.iter_at_offset(caret));
    }

    /// Refreshes the match counter next to the search entry.
    fn update_label(&self) {
        let imp = self.imp();
        let w = self.widgets();
        let text = if let Some(msg) = imp.message.borrow().clone() {
            msg
        } else if !imp.visible.get() || w.entry.text().is_empty() {
            String::new()
        } else {
            let matches = imp.matches.borrow();
            if matches.is_empty() {
                "Aucun résultat".to_owned()
            } else {
                let total = if imp.capped.get() {
                    "10 000+".to_owned()
                } else {
                    matches.len().to_string()
                };
                match imp.current.get().filter(|&i| i < matches.len()) {
                    Some(i) => format!("{} sur {total}", i + 1),
                    None => format!("{total} résultats"),
                }
            }
        };
        w.label.set_text(&text);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// All GTK checks live in one test: GTK can only be used from one thread.
    // Run from crate::gtk_tests (GTK is single-threaded).
    pub(crate) fn gtk_search_bar() {
        if gtk::init().is_err() {
            return;
        }
        let bar = SearchBar::new();
        bar.set_editor(None);
        bar.show(false);
        bar.show(true);
        bar.hide();
        bar.hide();
        assert!(!bar.imp().visible.get());
    }
}
