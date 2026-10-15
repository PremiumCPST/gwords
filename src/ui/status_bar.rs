//! Bottom status bar: word/character counts, cursor position.

use gtk::glib;
use gtk::subclass::prelude::*;

use crate::editor::Editor;

/// Delay after the last buffer change before recounting.
const DEBOUNCE_MS: u64 = 120;
/// Duration of the count tween.
const TWEEN_MS: u32 = 250;
/// Larger jumps are set directly instead of animated.
const TWEEN_MAX_DIFF: f64 = 5000.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Unit {
    Words,
    Chars,
}

/// Groups digits by thousands with U+202F (narrow no-break space), French style.
fn group_thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push('\u{202F}');
        }
        out.push(c);
    }
    out
}

fn format_count(n: usize, unit: Unit) -> String {
    let num = group_thousands(n);
    match (unit, n) {
        (Unit::Words, 1) => "1 mot".to_owned(),
        (Unit::Words, _) => format!("{num} mots"),
        (Unit::Chars, 1) => "1 caractère".to_owned(),
        (Unit::Chars, _) => format!("{num} caractères"),
    }
}

mod imp {
    use super::*;
    use std::cell::{Cell, OnceCell, RefCell};
    use std::rc::Rc;
    use std::time::Duration;

    use adw::prelude::AnimationExt;
    use gtk::prelude::*;

    struct Labels {
        cursor: gtk::Label,
        words: gtk::Label,
        chars: gtk::Label,
    }

    /// Live connection to the editor's buffer, undone on rebind.
    struct Binding {
        editor: glib::WeakRef<Editor>,
        buffer: glib::WeakRef<gtk::TextBuffer>,
        handlers: Vec<glib::SignalHandlerId>,
        /// Edits inside table cells don't touch the main buffer: the editor's
        /// format-changed signal (re-emitted for cells) triggers a recount.
        editor_handler: Option<glib::SignalHandlerId>,
    }

    /// Animated display of one count.
    #[derive(Default)]
    struct Tween {
        shown: Rc<Cell<f64>>,
        anim: RefCell<Option<adw::TimedAnimation>>,
    }

    impl Tween {
        fn show(&self, label: &gtk::Label, unit: Unit, target: usize) {
            if let Some(old) = self.anim.take() {
                old.pause();
            }
            let to = target as f64;
            let from = self.shown.get();
            if from == to || (to - from).abs() > TWEEN_MAX_DIFF {
                self.shown.set(to);
                label.set_text(&format_count(target, unit));
                return;
            }

            let shown = self.shown.clone();
            let weak = label.downgrade();
            let callback_target = adw::CallbackAnimationTarget::new(move |v| {
                shown.set(v);
                if let Some(label) = weak.upgrade() {
                    label.set_text(&format_count(v.round().max(0.0) as usize, unit));
                }
            });
            let anim = adw::TimedAnimation::new(label, from, to, TWEEN_MS, callback_target);
            anim.set_easing(adw::Easing::EaseOutCubic);
            anim.play();
            *self.anim.borrow_mut() = Some(anim);
        }

        fn clear(&self, label: &gtk::Label) {
            if let Some(old) = self.anim.take() {
                old.pause();
            }
            self.shown.set(0.0);
            label.set_text("");
        }
    }

    #[derive(Default)]
    pub struct StatusBar {
        labels: OnceCell<Labels>,
        words: Tween,
        chars: Tween,
        binding: RefCell<Option<Binding>>,
        debounce: Cell<Option<glib::SourceId>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for StatusBar {
        const NAME: &'static str = "GwStatusBar";
        type Type = super::StatusBar;
        type ParentType = gtk::Box;
    }

    impl ObjectImpl for StatusBar {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            obj.set_orientation(gtk::Orientation::Horizontal);
            obj.set_spacing(12);
            obj.set_margin_start(12);
            obj.set_margin_end(12);
            obj.set_margin_top(6);
            obj.set_margin_bottom(6);
            obj.add_css_class("status-bar");

            let make = || {
                let label = gtk::Label::new(None);
                label.add_css_class("numeric");
                label.add_css_class("dim-label");
                label.add_css_class("caption");
                label
            };
            let cursor = make();
            cursor.set_xalign(0.0);
            let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            spacer.set_hexpand(true);
            let words = make();
            let chars = make();

            obj.append(&cursor);
            obj.append(&spacer);
            obj.append(&words);
            obj.append(&chars);

            let labels = Labels {
                cursor,
                words,
                chars,
            };
            labels.words.set_text(&format_count(0, Unit::Words));
            labels.chars.set_text(&format_count(0, Unit::Chars));
            let _ = self.labels.set(labels);
        }

        fn dispose(&self) {
            self.unbind();
        }
    }

    impl WidgetImpl for StatusBar {}
    impl BoxImpl for StatusBar {}

    impl StatusBar {
        fn labels(&self) -> &Labels {
            self.labels.get().expect("status bar constructed")
        }

        /// Disconnects from the previous editor (if any) and connects to the new one.
        pub(super) fn bind(&self, editor: Option<&Editor>) {
            self.unbind();

            let Some(editor) = editor else {
                let labels = self.labels();
                self.words.clear(&labels.words);
                self.chars.clear(&labels.chars);
                labels.cursor.set_text("");
                return;
            };

            let buffer = editor.buffer();
            let obj = self.obj();
            let handlers = vec![
                buffer.connect_changed(glib::clone!(
                    #[weak]
                    obj,
                    move |_| obj.imp().schedule_counts()
                )),
                buffer.connect_mark_set(glib::clone!(
                    #[weak]
                    obj,
                    move |buf, _, _| obj.imp().update_cursor(buf)
                )),
                buffer.connect_notify_local(
                    Some("cursor-position"),
                    glib::clone!(
                        #[weak]
                        obj,
                        move |buf, _| obj.imp().update_cursor(buf)
                    ),
                ),
            ];

            *self.binding.borrow_mut() = Some(Binding {
                editor: editor.downgrade(),
                buffer: buffer.downgrade(),
                handlers,
                editor_handler: Some(editor.connect_format_changed(glib::clone!(
                    #[weak]
                    obj,
                    move |_| obj.imp().schedule_counts()
                ))),
            });
            self.update_cursor(&buffer);
            self.refresh_counts();
        }

        /// Removes the handlers connected by `bind` and any pending recount.
        fn unbind(&self) {
            if let Some(mut binding) = self.binding.take() {
                if let (Some(editor), Some(id)) = (binding.editor.upgrade(), binding.editor_handler.take()) {
                    editor.disconnect(id);
                }
                if let Some(buffer) = binding.buffer.upgrade() {
                    for handler in binding.handlers {
                        buffer.disconnect(handler);
                    }
                }
            }
            if let Some(id) = self.debounce.take() {
                id.remove();
            }
        }

        fn update_cursor(&self, buffer: &gtk::TextBuffer) {
            let iter = buffer.iter_at_offset(buffer.cursor_position());
            let line = iter.line() + 1;
            let column = iter.line_offset() + 1;
            self.labels()
                .cursor
                .set_text(&format!("Ligne {line}, colonne {column}"));
        }

        /// Restarts the debounce timer; the recount runs once typing pauses.
        fn schedule_counts(&self) {
            if let Some(id) = self.debounce.take() {
                id.remove();
            }
            let weak = self.obj().downgrade();
            let id = glib::timeout_add_local_once(Duration::from_millis(DEBOUNCE_MS), move || {
                if let Some(obj) = weak.upgrade() {
                    let imp = obj.imp();
                    let _ = imp.debounce.take();
                    imp.refresh_counts();
                }
            });
            self.debounce.set(Some(id));
        }

        fn refresh_counts(&self) {
            let editor = self
                .binding
                .borrow()
                .as_ref()
                .and_then(|b| b.editor.upgrade());
            let Some(editor) = editor else {
                return;
            };
            let (words, chars) = editor.counts();
            let labels = self.labels();
            self.words.show(&labels.words, Unit::Words, words);
            self.chars.show(&labels.chars, Unit::Chars, chars);
        }
    }
}

glib::wrapper! {
    pub struct StatusBar(ObjectSubclass<imp::StatusBar>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl Default for StatusBar {
    fn default() -> Self {
        Self::new()
    }
}

impl StatusBar {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Binds to the editor of the active tab (`None` when no tab is open).
    /// Must disconnect handlers from the previous editor.
    pub fn set_editor(&self, editor: Option<&Editor>) {
        self.imp().bind(editor);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thousands_grouping_uses_narrow_nbsp() {
        assert_eq!(group_thousands(0), "0");
        assert_eq!(group_thousands(999), "999");
        assert_eq!(group_thousands(1234), "1\u{202F}234");
        assert_eq!(group_thousands(1234567), "1\u{202F}234\u{202F}567");
    }

    #[test]
    fn count_labels_are_singular_or_plural() {
        assert_eq!(format_count(1, Unit::Words), "1 mot");
        assert_eq!(format_count(0, Unit::Words), "0 mots");
        assert_eq!(format_count(1, Unit::Chars), "1 caractère");
        assert_eq!(format_count(6789, Unit::Chars), "6\u{202F}789 caractères");
    }
}
