//! Formatting toolbar shown under the header bar.

use std::cell::{Cell, OnceCell, RefCell};
use std::time::Duration;

use gtk::glib;
use gtk::pango;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use crate::document::{Alignment, ListKind, ParagraphStyle, Rgb, VerticalAlign};
use crate::editor::{Editor, FormatState};

const DEFAULT_SIZE: f64 = 12.0;

pub struct Controls {
    style: gtk::DropDown,
    font: gtk::FontDialogButton,
    size: gtk::SpinButton,
    bold: gtk::ToggleButton,
    italic: gtk::ToggleButton,
    underline: gtk::ToggleButton,
    strike: gtk::ToggleButton,
    sup: gtk::ToggleButton,
    sub: gtk::ToggleButton,
    color: gtk::ColorDialogButton,
    highlight: gtk::ColorDialogButton,
    clear: gtk::Button,
    aligns: [(Alignment, gtk::ToggleButton); 4],
    bullet: gtk::ToggleButton,
    numbered: gtk::ToggleButton,
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct FormatBar {
        pub controls: OnceCell<Controls>,
        pub updating: Cell<bool>,
        pub editor: glib::WeakRef<Editor>,
        pub handler: RefCell<Option<glib::SignalHandlerId>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for FormatBar {
        const NAME: &'static str = "GwFormatBar";
        type Type = super::FormatBar;
        type ParentType = gtk::Box;
    }

    impl ObjectImpl for FormatBar {}
    impl WidgetImpl for FormatBar {}
    impl BoxImpl for FormatBar {}
}

glib::wrapper! {
    pub struct FormatBar(ObjectSubclass<imp::FormatBar>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl Default for FormatBar {
    fn default() -> Self {
        Self::new()
    }
}

fn rgba_to_rgb(c: &gtk::gdk::RGBA) -> Rgb {
    let f = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Rgb(f(c.red()), f(c.green()), f(c.blue()))
}

fn rgb_to_rgba(c: Rgb) -> gtk::gdk::RGBA {
    gtk::gdk::RGBA::new(c.0 as f32 / 255.0, c.1 as f32 / 255.0, c.2 as f32 / 255.0, 1.0)
}

fn toggle(icon: Option<&str>, label: Option<&str>, tip: &str) -> gtk::ToggleButton {
    let b = gtk::ToggleButton::new();
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

fn linked(buttons: &[&gtk::ToggleButton]) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    b.add_css_class("linked");
    for w in buttons {
        b.append(*w);
    }
    b
}

fn separator() -> gtk::Separator {
    let s = gtk::Separator::new(gtk::Orientation::Vertical);
    s.add_css_class("spacer");
    s
}

fn pop(button: &gtk::ToggleButton) {
    button.add_css_class("pop");
    let weak = button.downgrade();
    glib::timeout_add_local_once(Duration::from_millis(200), move || {
        if let Some(b) = weak.upgrade() {
            b.remove_css_class("pop");
        }
    });
}

impl FormatBar {
    pub fn new() -> Self {
        let bar: Self = glib::Object::new();
        bar.build();
        bar.set_sensitive(false);
        bar
    }

    fn build(&self) {
        self.add_css_class("toolbar");
        self.add_css_class("format-bar");
        self.set_spacing(6);

        let labels: Vec<&str> = ParagraphStyle::ALL.iter().map(|s| s.label()).collect();
        let style = gtk::DropDown::from_strings(&labels);
        style.set_tooltip_text(Some("Style de paragraphe"));

        let dialog = gtk::FontDialog::new();
        let font = gtk::FontDialogButton::new(Some(dialog));
        font.set_level(gtk::FontLevel::Family);
        font.set_use_font(true);
        font.set_tooltip_text(Some("Police"));

        let size = gtk::SpinButton::with_range(1.0, 200.0, 1.0);
        size.set_digits(0);
        size.set_width_chars(3);
        size.set_value(DEFAULT_SIZE);
        size.set_tooltip_text(Some("Taille"));

        let bold = toggle(Some("format-text-bold-symbolic"), None, "Gras (Ctrl+B)");
        let italic = toggle(Some("format-text-italic-symbolic"), None, "Italique (Ctrl+I)");
        let underline = toggle(
            Some("format-text-underline-symbolic"),
            None,
            "Souligné (Ctrl+U)",
        );
        let strike = toggle(
            Some("format-text-strikethrough-symbolic"),
            None,
            "Barré",
        );
        let sup = toggle(None, Some("x²"), "Exposant");
        let sub = toggle(None, Some("x₂"), "Indice");

        let color = gtk::ColorDialogButton::new(Some(gtk::ColorDialog::new()));
        color.set_tooltip_text(Some("Couleur du texte"));
        color.set_rgba(&gtk::gdk::RGBA::BLACK);
        let highlight = gtk::ColorDialogButton::new(Some(gtk::ColorDialog::new()));
        highlight.set_tooltip_text(Some("Surlignage"));
        highlight.set_rgba(&gtk::gdk::RGBA::new(1.0, 1.0, 0.0, 1.0));

        let clear = gtk::Button::from_icon_name("edit-clear-all-symbolic");
        clear.set_tooltip_text(Some("Effacer la mise en forme"));
        clear.add_css_class("flat");

        let al = toggle(Some("format-justify-left-symbolic"), None, "Aligner à gauche");
        let ac = toggle(Some("format-justify-center-symbolic"), None, "Centrer");
        let ar = toggle(Some("format-justify-right-symbolic"), None, "Aligner à droite");
        let aj = toggle(Some("format-justify-fill-symbolic"), None, "Justifier");
        ac.set_group(Some(&al));
        ar.set_group(Some(&al));
        aj.set_group(Some(&al));
        al.set_active(true);

        let bullet = toggle(Some("view-list-bullet-symbolic"), None, "Liste à puces");
        let numbered = toggle(
            Some("view-list-ordered-symbolic"),
            None,
            "Liste numérotée",
        );

        let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        content.set_halign(gtk::Align::Center);
        content.set_hexpand(true);
        content.set_margin_start(6);
        content.set_margin_end(6);
        content.append(&style);
        content.append(&separator());
        content.append(&font);
        content.append(&size);
        content.append(&separator());
        content.append(&linked(&[&bold, &italic, &underline, &strike]));
        content.append(&linked(&[&sup, &sub]));
        content.append(&separator());
        content.append(&color);
        content.append(&highlight);
        content.append(&clear);
        content.append(&separator());
        content.append(&linked(&[&al, &ac, &ar, &aj]));
        content.append(&separator());
        content.append(&linked(&[&bullet, &numbered]));

        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::External)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .hexpand(true)
            .child(&content)
            .build();
        self.append(&scroller);

        let controls = Controls {
            style,
            font,
            size,
            bold,
            italic,
            underline,
            strike,
            sup,
            sub,
            color,
            highlight,
            clear,
            aligns: [
                (Alignment::Left, al),
                (Alignment::Center, ac),
                (Alignment::Right, ar),
                (Alignment::Justify, aj),
            ],
            bullet,
            numbered,
        };
        let _ = self.imp().controls.set(controls);
        self.wire();
    }

    /// Runs `f` with the bound editor unless a sync is in progress, then
    /// returns focus to the editor.
    fn act(&self, refocus: bool, f: impl FnOnce(&Editor)) {
        let imp = self.imp();
        if imp.updating.get() {
            return;
        }
        if let Some(editor) = imp.editor.upgrade() {
            f(&editor);
            if refocus {
                editor.grab_focus();
            }
        }
    }

    fn wire(&self) {
        let c = self.imp().controls.get().expect("controls built");

        let simple = |b: &gtk::ToggleButton, f: fn(&Editor)| {
            let bar = self.downgrade();
            b.connect_toggled(move |b| {
                if let Some(bar) = bar.upgrade() {
                    if b.is_active() && !bar.imp().updating.get() {
                        pop(b);
                    }
                    bar.act(true, f);
                }
            });
        };
        simple(&c.bold, |e| e.toggle_bold());
        simple(&c.italic, |e| e.toggle_italic());
        simple(&c.underline, |e| e.toggle_underline());
        simple(&c.strike, |e| e.toggle_strikethrough());

        let bar = self.downgrade();
        c.style.connect_selected_notify(move |d| {
            if let Some(bar) = bar.upgrade() {
                let idx = d.selected() as usize;
                if let Some(s) = ParagraphStyle::ALL.get(idx).copied() {
                    bar.act(true, |e| e.set_paragraph_style(s));
                }
            }
        });

        let bar = self.downgrade();
        c.font.connect_font_desc_notify(move |b| {
            if let Some(bar) = bar.upgrade() {
                let family = b
                    .font_desc()
                    .and_then(|d| d.family().map(|f| f.to_string()));
                if let Some(family) = family {
                    bar.act(true, |e| e.set_font_family(Some(&family)));
                }
            }
        });

        let bar = self.downgrade();
        c.size.connect_value_changed(move |s| {
            if let Some(bar) = bar.upgrade() {
                let v = s.value();
                bar.act(false, |e| e.set_font_size(Some(v)));
            }
        });
        let bar = self.downgrade();
        c.size.connect_activate(move |_| {
            if let Some(bar) = bar.upgrade() {
                bar.act(true, |_| {});
            }
        });

        // Superscript / subscript: mutually exclusive.
        for (me, other, kind) in [
            (&c.sup, &c.sub, VerticalAlign::Superscript),
            (&c.sub, &c.sup, VerticalAlign::Subscript),
        ] {
            let bar = self.downgrade();
            let other = other.downgrade();
            me.connect_toggled(move |b| {
                let Some(bar) = bar.upgrade() else { return };
                if bar.imp().updating.get() {
                    return;
                }
                let active = b.is_active();
                if active {
                    pop(b);
                    if let Some(o) = other.upgrade() {
                        bar.imp().updating.set(true);
                        o.set_active(false);
                        bar.imp().updating.set(false);
                    }
                }
                let v = if active { kind } else { VerticalAlign::Baseline };
                bar.act(true, |e| e.set_vertical_align(v));
            });
        }

        let bar = self.downgrade();
        c.color.connect_rgba_notify(move |b| {
            if let Some(bar) = bar.upgrade() {
                let rgb = rgba_to_rgb(&b.rgba());
                bar.act(true, |e| e.set_text_color(Some(rgb)));
            }
        });
        let bar = self.downgrade();
        c.highlight.connect_rgba_notify(move |b| {
            if let Some(bar) = bar.upgrade() {
                let rgb = rgba_to_rgb(&b.rgba());
                bar.act(true, |e| e.set_highlight(Some(rgb)));
            }
        });

        let bar = self.downgrade();
        c.clear.connect_clicked(move |_| {
            if let Some(bar) = bar.upgrade() {
                bar.act(true, |e| e.clear_formatting());
            }
        });

        for (a, b) in &c.aligns {
            let bar = self.downgrade();
            let a = *a;
            b.connect_toggled(move |b| {
                if let Some(bar) = bar.upgrade() {
                    if b.is_active() {
                        if !bar.imp().updating.get() {
                            pop(b);
                        }
                        bar.act(true, |e| e.set_alignment(a));
                    }
                }
            });
        }

        for (b, kind) in [(&c.bullet, ListKind::Bullet), (&c.numbered, ListKind::Numbered)] {
            let bar = self.downgrade();
            b.connect_toggled(move |b| {
                if let Some(bar) = bar.upgrade() {
                    if b.is_active() && !bar.imp().updating.get() {
                        pop(b);
                    }
                    bar.act(true, |e| e.toggle_list(kind));
                }
            });
        }
    }

    /// Binds to the editor of the active tab (`None` when no tab is open).
    /// Must disconnect handlers from the previous editor.
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
                        bar.sync(&ed.format_state());
                    }
                });
                imp.handler.replace(Some(id));
                self.set_sensitive(true);
                self.sync(&ed.format_state());
            }
            None => self.set_sensitive(false),
        }
    }

    /// Updates every control from `st` without triggering editor actions.
    fn sync(&self, st: &FormatState) {
        let imp = self.imp();
        let Some(c) = imp.controls.get() else { return };
        imp.updating.set(true);

        if let Some(i) = ParagraphStyle::ALL.iter().position(|s| *s == st.paragraph_style) {
            c.style.set_selected(i as u32);
        }
        let defaults = crate::settings::get();
        let mut desc = pango::FontDescription::new();
        desc.set_family(st.font_family.as_deref().unwrap_or(&defaults.font_family));
        c.font.set_font_desc(&desc);
        c.size.set_value(st.font_size.unwrap_or(defaults.font_size));

        c.bold.set_active(st.bold);
        c.italic.set_active(st.italic);
        c.underline.set_active(st.underline);
        c.strike.set_active(st.strikethrough);
        c.sup.set_active(st.vertical == VerticalAlign::Superscript);
        c.sub.set_active(st.vertical == VerticalAlign::Subscript);

        // `None` = default text color / no highlight (shown as transparent).
        c.color.set_rgba(&st.color.map(rgb_to_rgba).unwrap_or(gtk::gdk::RGBA::new(0.0, 0.0, 0.0, 0.0)));
        {
            c.highlight.set_rgba(&st.highlight.map(rgb_to_rgba).unwrap_or(gtk::gdk::RGBA::new(0.0, 0.0, 0.0, 0.0)));
        }

        for (a, b) in &c.aligns {
            if *a == st.alignment {
                b.set_active(true);
            }
        }
        c.bullet.set_active(st.list == Some(ListKind::Bullet));
        c.numbered.set_active(st.list == Some(ListKind::Numbered));

        imp.updating.set(false);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    // Run from crate::gtk_tests (GTK is single-threaded).
    pub(crate) fn gtk_checks() {
        // Pure conversion checks.
        assert_eq!(rgba_to_rgb(&rgb_to_rgba(Rgb(12, 200, 255))), Rgb(12, 200, 255));
        if gtk::init().is_err() {
            return;
        }
        let bar = FormatBar::new();
        assert!(!bar.is_sensitive());
        bar.set_editor(None);
        assert!(bar.has_css_class("format-bar"));
    }
}
