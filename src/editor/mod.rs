//! The rich text editor widget: a `gtk::TextView` subclass.
//!
//! PUBLIC API CONTRACT (used by window, document_page, ui/*): keep these
//! signatures stable.

pub mod caret_anim;
pub mod convert;
pub mod tags;
pub mod typing_anim;

use gtk::glib;
use gtk::glib::subclass::Signal;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use std::cell::{Cell, RefCell};
use std::sync::OnceLock;

use crate::theme;
use tags::*;

use crate::document::{Alignment, Document, ListKind, ParagraphStyle, Rgb, VerticalAlign};

/// Formatting at the cursor (or of the selection: a flag is true only if it
/// applies to the whole selection). Used by the format bar to sync its buttons.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FormatState {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
    pub vertical: VerticalAlign,
    pub paragraph_style: ParagraphStyle,
    pub alignment: Alignment,
    pub list: Option<ListKind>,
    pub font_family: Option<String>,
    pub font_size: Option<f64>,
    pub color: Option<Rgb>,
    pub highlight: Option<Rgb>,
}


// ---------------------------------------------------------------------------
// Buffer helpers (free functions)
// ---------------------------------------------------------------------------

fn is_char_tag(name: &str) -> bool {
    matches!(name, BOLD | ITALIC | UNDERLINE | STRIKETHROUGH | SUPERSCRIPT | SUBSCRIPT)
        || name.starts_with(FONT_PREFIX)
        || name.starts_with(SIZE_PREFIX)
        || name.starts_with(COLOR_PREFIX)
        || name.starts_with(HIGHLIGHT_PREFIX)
}

fn is_para_tag(name: &str) -> bool {
    name.starts_with("para-")
        || name.starts_with("align-")
        || (name.starts_with("list-") && name != LIST_MARKER)
}

fn is_heading_tag(name: &str) -> bool {
    matches!(name, PARA_TITLE | PARA_H1 | PARA_H2 | PARA_H3)
}

fn tag_names_at(it: &gtk::TextIter) -> Vec<String> {
    it.tags().iter().filter_map(|t| t.name().map(|s| s.to_string())).collect()
}

fn marker_tag(b: &gtk::TextBuffer) -> gtk::TextTag {
    b.tag_table().lookup(LIST_MARKER).expect("tags installed")
}

fn line_start(b: &gtk::TextBuffer, line: i32) -> gtk::TextIter {
    b.iter_at_line(line).unwrap_or_else(|| b.end_iter())
}

/// Offsets of a line including its trailing newline.
fn line_bounds(b: &gtk::TextBuffer, line: i32) -> (i32, i32) {
    let mut it = line_start(b, line);
    let s = it.offset();
    if !it.forward_line() {
        return (s, b.end_iter().offset());
    }
    (s, it.offset())
}

/// Offset just after the list marker of the line (== line start if none).
fn marker_end(b: &gtk::TextBuffer, line: i32) -> i32 {
    let ls = line_start(b, line);
    let tag = marker_tag(b);
    if ls.has_tag(&tag) {
        let mut it = ls;
        it.forward_to_tag_toggle(Some(&tag));
        it.offset()
    } else {
        ls.offset()
    }
}

fn line_end_no_nl(b: &gtk::TextBuffer, line: i32) -> i32 {
    let mut it = line_start(b, line);
    if !it.ends_line() {
        it.forward_to_line_end();
    }
    it.offset()
}

/// Sub-ranges of [s, e) that are plain text (no markers, no newlines).
fn segments(b: &gtk::TextBuffer, s: i32, e: i32) -> Vec<(i32, i32)> {
    let mut out = Vec::new();
    if s >= e {
        return out;
    }
    let l0 = b.iter_at_offset(s).line();
    let l1 = b.iter_at_offset(e).line();
    for l in l0..=l1 {
        let ls = line_start(b, l).offset();
        let a = s.max(marker_end(b, l).max(ls));
        let z = e.min(line_end_no_nl(b, l));
        if a < z {
            out.push((a, z));
        }
    }
    out
}

fn tags_matching(b: &gtk::TextBuffer, pred: impl Fn(&str) -> bool) -> Vec<gtk::TextTag> {
    let mut v = Vec::new();
    b.tag_table().foreach(|t| {
        if t.name().map(|n| pred(&n)).unwrap_or(false) {
            v.push(t.clone());
        }
    });
    v
}

fn remove_tags_range(b: &gtk::TextBuffer, tags: &[gtk::TextTag], s: i32, e: i32) {
    let (si, ei) = (b.iter_at_offset(s), b.iter_at_offset(e));
    for t in tags {
        b.remove_tag(t, &si, &ei);
    }
}

fn apply_named(b: &gtk::TextBuffer, name: &str, add: bool, s: i32, e: i32) {
    if s >= e {
        return;
    }
    if let Some(tag) = ensure_tag(b, name) {
        let (si, ei) = (b.iter_at_offset(s), b.iter_at_offset(e));
        if add {
            b.apply_tag(&tag, &si, &ei);
        } else {
            b.remove_tag(&tag, &si, &ei);
        }
    }
}

fn fully_tagged(b: &gtk::TextBuffer, name: &str, segs: &[(i32, i32)]) -> bool {
    let Some(tag) = b.tag_table().lookup(name) else { return false };
    segs.iter().all(|&(s, e)| {
        let it = b.iter_at_offset(s);
        if !it.has_tag(&tag) {
            return false;
        }
        let mut it2 = it;
        if it2.forward_to_tag_toggle(Some(&tag)) {
            it2.offset() >= e
        } else {
            true
        }
    })
}

fn plain_char_at(b: &gtk::TextBuffer, off: i32) -> Option<gtk::TextIter> {
    if off < 0 || off >= b.char_count() {
        return None;
    }
    let it = b.iter_at_offset(off);
    if it.char() == '\n' || it.has_tag(&marker_tag(b)) {
        None
    } else {
        Some(it)
    }
}

/// Char-format tags a text inserted at [start, end) should inherit.
fn inherit_char_tags(b: &gtk::TextBuffer, start: i32, end: i32) -> Vec<String> {
    let it = plain_char_at(b, start - 1).or_else(|| plain_char_at(b, end));
    it.map(|it| tag_names_at(&it).into_iter().filter(|n| is_char_tag(n)).collect()).unwrap_or_default()
}

/// Paragraph tags of the line a text inserted at [start, end) belongs to.
fn para_source_tags(b: &gtk::TextBuffer, start: i32, end: i32) -> Vec<String> {
    let src = if start > 0 && b.iter_at_offset(start - 1).char() != '\n' {
        Some(start - 1)
    } else if end < b.char_count() {
        Some(end)
    } else {
        None
    };
    src.map(|o| tag_names_at(&b.iter_at_offset(o)).into_iter().filter(|n| is_para_tag(n)).collect())
        .unwrap_or_default()
}

fn selection_range(b: &gtk::TextBuffer) -> Option<(i32, i32)> {
    b.selection_bounds().map(|(a, z)| (a.offset(), z.offset()))
}

fn cursor_offset(b: &gtk::TextBuffer) -> i32 {
    b.iter_at_mark(&b.get_insert()).offset()
}

/// Lines touched by the selection (or the cursor line).
fn selected_lines(b: &gtk::TextBuffer) -> (i32, i32) {
    let (s, e) = match b.selection_bounds() {
        Some(p) => p,
        None => {
            let i = b.iter_at_mark(&b.get_insert());
            (i, i)
        }
    };
    let l0 = s.line();
    let mut l1 = e.line();
    if e.line_offset() == 0 && l1 > l0 {
        l1 -= 1;
    }
    (l0, l1)
}

fn list_kind(b: &gtk::TextBuffer, line: i32) -> Option<ListKind> {
    tag_names_at(&line_start(b, line)).iter().find_map(|n| list_from_tag(n))
}

fn insert_marker(b: &gtk::TextBuffer, at: i32, text: &str, kind: ListKind) {
    let mut it = b.iter_at_offset(at);
    b.insert(&mut it, text);
    let end = at + text.chars().count() as i32;
    let (si, ei) = (b.iter_at_offset(at), b.iter_at_offset(end));
    b.apply_tag(&marker_tag(b), &si, &ei);
    if let Some(t) = b.tag_table().lookup(list_tag(kind)) {
        b.apply_tag(&t, &si, &ei);
    }
}

fn strip_marker(b: &gtk::TextBuffer, line: i32) {
    let ls = line_start(b, line);
    if ls.has_tag(&marker_tag(b)) {
        let me = marker_end(b, line);
        let mut s = b.iter_at_offset(ls.offset());
        let mut e = b.iter_at_offset(me);
        b.delete(&mut s, &mut e);
    }
}

fn clear_list_line(b: &gtk::TextBuffer, line: i32) {
    strip_marker(b, line);
    let (s, e) = line_bounds(b, line);
    let tags = tags_matching(b, |n| list_from_tag(n).is_some());
    remove_tags_range(b, &tags, s, e);
}

fn set_list_line(b: &gtk::TextBuffer, line: i32, kind: ListKind) {
    clear_list_line(b, line);
    let ls = line_start(b, line).offset();
    let text = match kind {
        ListKind::Bullet => BULLET_MARKER.to_string(),
        ListKind::Numbered => numbered_marker(1),
    };
    insert_marker(b, ls, &text, kind);
    let (s, e) = line_bounds(b, line);
    apply_named(b, list_tag(kind), true, s, e);
}

fn renumber(b: &gtk::TextBuffer) {
    let mut n = 0usize;
    for l in 0..b.line_count() {
        if list_kind(b, l) == Some(ListKind::Numbered) {
            n += 1;
            let ls = line_start(b, l);
            if !ls.has_tag(&marker_tag(b)) {
                continue;
            }
            let me = marker_end(b, l);
            let cur = b.text(&ls, &b.iter_at_offset(me), false);
            let want = numbered_marker(n);
            if cur.as_str() != want {
                let so = ls.offset();
                let mut s = b.iter_at_offset(so);
                let mut e = b.iter_at_offset(me);
                b.delete(&mut s, &mut e);
                insert_marker(b, so, &want, ListKind::Numbered);
            }
        } else {
            n = 0;
        }
    }
}

// ---------------------------------------------------------------------------

mod imp {
    use super::*;

    pub struct Editor {
        pub typing: typing_anim::TypingAnimator,
        pub caret: caret_anim::CaretAnimator,
        /// Full set of char-format tags for the next typed text.
        pub pending: RefCell<Option<Vec<String>>>,
        pub pending_pos: Cell<i32>,
        pub loading: Cell<bool>,
        pub internal: Cell<bool>,
        pub caret_anim: Cell<bool>,
        pub css_class: RefCell<Option<String>>,
        pub provider: RefCell<Option<gtk::CssProvider>>,
    }

    impl Default for Editor {
        fn default() -> Self {
            Self {
                typing: typing_anim::TypingAnimator::new(),
                caret: caret_anim::CaretAnimator::new(),
                pending: RefCell::new(None),
                pending_pos: Cell::new(0),
                loading: Cell::new(false),
                internal: Cell::new(false),
                caret_anim: Cell::new(true),
                css_class: RefCell::new(None),
                provider: RefCell::new(None),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Editor {
        const NAME: &'static str = "GwEditor";
        type Type = super::Editor;
        type ParentType = gtk::TextView;
    }

    impl ObjectImpl for Editor {
        fn signals() -> &'static [Signal] {
            static SIGNALS: OnceLock<Vec<Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| vec![Signal::builder("format-changed").build()])
        }

        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            let buffer = obj.buffer();
            install_tags(&buffer);
            buffer.set_enable_undo(true);
            obj.set_wrap_mode(gtk::WrapMode::WordChar);
            obj.set_left_margin(theme::PAGE_PADDING_X);
            obj.set_right_margin(theme::PAGE_PADDING_X);
            obj.set_top_margin(theme::PAGE_PADDING_Y);
            obj.set_bottom_margin(theme::PAGE_PADDING_Y);
            obj.set_pixels_below_lines(4);
            obj.set_accepts_tab(true);
            obj.add_css_class("page");
            obj.add_css_class("custom-caret");
            self.typing.set_enabled(true);
            self.caret.set_enabled(true);
            self.typing.attach(obj.upcast_ref());
            self.caret.attach(obj.upcast_ref());

            // Typing inheritance (after the default handler).
            let weak = obj.downgrade();
            let closure = glib::RustClosure::new_local(move |args| {
                let obj = weak.upgrade()?;
                let iter = args[1].get::<gtk::TextIter>().ok()?;
                let text = args[2].get::<String>().ok()?;
                let end = iter.offset();
                let start = end - text.chars().count() as i32;
                obj.after_insert(start, end, text.contains('\n'));
                None
            });
            buffer.connect_closure("insert-text", true, closure);

            let weak = obj.downgrade();
            buffer.connect_mark_set(move |_, iter, mark| {
                if let Some(obj) = weak.upgrade() {
                    obj.on_mark_set(iter, mark);
                }
            });
            let weak = obj.downgrade();
            buffer.connect_changed(move |_| {
                if let Some(obj) = weak.upgrade() {
                    obj.notify_format();
                }
            });
            let weak = obj.downgrade();
            buffer.connect_end_user_action(move |b| {
                if let Some(obj) = weak.upgrade() {
                    if !obj.imp().internal.get() && !obj.imp().loading.get() {
                        obj.imp().internal.set(true);
                        renumber(b);
                        obj.imp().internal.set(false);
                    }
                }
            });

            let key = gtk::EventControllerKey::new();
            key.set_propagation_phase(gtk::PropagationPhase::Capture);
            let weak = obj.downgrade();
            key.connect_key_pressed(move |_, keyval, _, state| {
                match weak.upgrade() {
                    Some(obj) if obj.on_key(keyval, state) => glib::Propagation::Stop,
                    _ => glib::Propagation::Proceed,
                }
            });
            obj.add_controller(key);
        }

        #[allow(deprecated)]
        fn dispose(&self) {
            if let Some(p) = self.provider.borrow_mut().take() {
                if let Some(d) = gtk::gdk::Display::default() {
                    gtk::style_context_remove_provider_for_display(&d, &p);
                }
            }
        }
    }

    impl WidgetImpl for Editor {}

    impl TextViewImpl for Editor {
        fn snapshot_layer(&self, layer: gtk::TextViewLayer, snapshot: gtk::Snapshot) {
            self.parent_snapshot_layer(layer, snapshot.clone());
            if layer == gtk::TextViewLayer::AboveText {
                let obj = self.obj();
                let view: &gtk::TextView = obj.upcast_ref();
                self.typing.snapshot_above(view, &snapshot);
                self.caret.snapshot_above(view, &snapshot);
            }
        }
    }
}

glib::wrapper! {
    pub struct Editor(ObjectSubclass<imp::Editor>)
        @extends gtk::TextView, gtk::Widget,
        @implements gtk::Accessible, gtk::AccessibleText, gtk::Buildable, gtk::ConstraintTarget, gtk::Scrollable;
}

impl Default for Editor {
    fn default() -> Self {
        Self::new()
    }
}

impl Editor {
    pub fn new() -> Self {
        glib::Object::new()
    }

    fn notify_format(&self) {
        if !self.imp().loading.get() {
            self.emit_by_name::<()>("format-changed", &[]);
        }
    }

    fn tags_changed(&self) {
        self.buffer().set_modified(true);
        self.notify_format();
    }

    /// Replaces the whole content. Resets undo history and clears the
    /// buffer's `modified` flag. Must not trigger typing animations.
    pub fn set_document(&self, doc: &Document) {
        let imp = self.imp();
        let buffer = self.buffer();
        imp.typing.set_suspended(true);
        imp.caret.set_suspended(true);
        imp.loading.set(true);
        *imp.pending.borrow_mut() = None;
        buffer.set_enable_undo(false);
        convert::load_into_buffer(&buffer, doc);
        buffer.place_cursor(&buffer.start_iter());
        buffer.set_enable_undo(true);
        buffer.set_modified(false);
        imp.loading.set(false);
        imp.typing.set_suspended(false);
        imp.caret.set_suspended(false);
        self.notify_format();
    }

    /// Snapshot of the current content as a document model.
    pub fn document(&self) -> Document {
        convert::buffer_to_document(&self.buffer())
    }

    // ----- internal event handling -----

    fn after_insert(&self, start: i32, end: i32, multiline: bool) {
        let imp = self.imp();
        if imp.loading.get() || imp.internal.get() || start >= end {
            return;
        }
        let b = self.buffer();
        let pending = imp.pending.borrow_mut().take();
        let char_tags = pending.unwrap_or_else(|| inherit_char_tags(&b, start, end));
        let para = para_source_tags(&b, start, end);
        if !multiline {
            for t in &para {
                apply_named(&b, t, true, start, end);
            }
        } else {
            let l0 = b.iter_at_offset(start).line();
            let l1 = b.iter_at_offset(end).line();
            let all = tags_matching(&b, is_para_tag);
            for l in l0..=l1 {
                let (s, e) = line_bounds(&b, l);
                remove_tags_range(&b, &all, s, e);
                for t in &para {
                    if l > l0 && (is_heading_tag(t) || list_from_tag(t).is_some()) {
                        continue;
                    }
                    apply_named(&b, t, true, s, e);
                }
            }
        }
        for (s, e) in segments(&b, start, end) {
            for t in &char_tags {
                apply_named(&b, t, true, s, e);
            }
        }
    }

    fn on_mark_set(&self, iter: &gtk::TextIter, mark: &gtk::TextMark) {
        let name = mark.name();
        let name = name.as_deref().unwrap_or("");
        if name != "insert" && name != "selection_bound" {
            return;
        }
        if name == "insert" {
            let b = self.buffer();
            let off = iter.offset();
            let line = iter.line();
            let ls = line_start(&b, line).offset();
            if off > ls {
                let me = marker_end(&b, line);
                if off < me {
                    b.move_mark(mark, &b.iter_at_offset(me));
                    return;
                }
            }
            let imp = self.imp();
            if imp.pending.borrow().is_some() && imp.pending_pos.get() != off {
                *imp.pending.borrow_mut() = None;
            }
        }
        self.notify_format();
    }

    fn on_key(&self, key: gtk::gdk::Key, state: gtk::gdk::ModifierType) -> bool {
        use gtk::gdk::Key;
        if state.intersects(gtk::gdk::ModifierType::CONTROL_MASK | gtk::gdk::ModifierType::ALT_MASK) {
            return false;
        }
        let is_enter = key == Key::Return || key == Key::KP_Enter;
        if !is_enter && key != Key::BackSpace {
            return false;
        }
        let b = self.buffer();
        if b.selection_bounds().is_some() {
            return false;
        }
        let ins = b.iter_at_mark(&b.get_insert());
        let line = ins.line();
        let Some(kind) = list_kind(&b, line) else { return false };
        let c = ins.offset();
        let lso = line_start(&b, line).offset();
        let me = marker_end(&b, line);
        let imp = self.imp();

        if key == Key::BackSpace {
            if c < lso || c > me || me == lso {
                return false;
            }
            b.begin_user_action();
            imp.internal.set(true);
            clear_list_line(&b, line);
            renumber(&b);
            imp.internal.set(false);
            b.end_user_action();
            self.tags_changed();
            return true;
        }

        let empty = line_end_no_nl(&b, line) <= me;
        b.begin_user_action();
        imp.internal.set(true);
        if c >= me && empty {
            clear_list_line(&b, line);
        } else if c <= lso {
            b.insert_at_cursor("\n");
        } else {
            let para: Vec<String> =
                tag_names_at(&line_start(&b, line)).into_iter().filter(|n| is_para_tag(n)).collect();
            b.insert_at_cursor("\n");
            for t in &para {
                apply_named(&b, t, true, c, c + 1);
            }
            let text = match kind {
                ListKind::Bullet => BULLET_MARKER.to_string(),
                ListKind::Numbered => numbered_marker(1),
            };
            insert_marker(&b, c + 1, &text, kind);
        }
        renumber(&b);
        imp.internal.set(false);
        b.end_user_action();
        self.tags_changed();
        self.scroll_mark_onscreen(&b.get_insert());
        true
    }

    // ----- character formatting -----

    fn selection_segments(&self) -> Option<Vec<(i32, i32)>> {
        let b = self.buffer();
        let (s, e) = selection_range(&b)?;
        let segs = segments(&b, s, e);
        if segs.is_empty() { None } else { Some(segs) }
    }

    fn pending_edit(&self, f: impl FnOnce(&mut Vec<String>)) {
        let b = self.buffer();
        let c = cursor_offset(&b);
        let imp = self.imp();
        let init = if imp.pending.borrow().is_none() { Some(inherit_char_tags(&b, c, c)) } else { None };
        {
            let mut p = imp.pending.borrow_mut();
            if let Some(i) = init {
                *p = Some(i);
            }
            f(p.as_mut().unwrap());
        }
        imp.pending_pos.set(c);
        self.notify_format();
    }

    fn toggle_char_tag(&self, name: &str) {
        let b = self.buffer();
        if let Some(segs) = self.selection_segments() {
            let all = fully_tagged(&b, name, &segs);
            b.begin_user_action();
            for (s, e) in segs {
                apply_named(&b, name, !all, s, e);
            }
            b.end_user_action();
            self.tags_changed();
        } else {
            self.pending_edit(|p| {
                if let Some(i) = p.iter().position(|n| n == name) {
                    p.remove(i);
                } else {
                    p.push(name.to_string());
                }
            });
        }
    }

    fn set_exclusive(&self, prefixes: &[&str], name: Option<String>) {
        let b = self.buffer();
        if let Some(segs) = self.selection_segments() {
            let old = tags_matching(&b, |n| prefixes.iter().any(|p| n.starts_with(p)));
            b.begin_user_action();
            for (s, e) in segs {
                remove_tags_range(&b, &old, s, e);
                if let Some(n) = &name {
                    apply_named(&b, n, true, s, e);
                }
            }
            b.end_user_action();
            self.tags_changed();
        } else {
            self.pending_edit(|p| {
                p.retain(|n| !prefixes.iter().any(|pf| n.starts_with(pf)));
                if let Some(n) = name {
                    ensure_tag(&b, &n);
                    p.push(n);
                }
            });
        }
    }

    pub fn toggle_bold(&self) { self.toggle_char_tag(BOLD) }
    pub fn toggle_italic(&self) { self.toggle_char_tag(ITALIC) }
    pub fn toggle_underline(&self) { self.toggle_char_tag(UNDERLINE) }
    pub fn toggle_strikethrough(&self) { self.toggle_char_tag(STRIKETHROUGH) }

    pub fn set_vertical_align(&self, v: VerticalAlign) {
        self.set_exclusive(&[SUPERSCRIPT, SUBSCRIPT], vertical_tag(v).map(String::from));
    }

    pub fn set_font_family(&self, family: Option<&str>) {
        self.set_exclusive(&[FONT_PREFIX], family.map(font_tag_name));
    }

    pub fn set_font_size(&self, points: Option<f64>) {
        self.set_exclusive(&[SIZE_PREFIX], points.map(size_tag_name));
    }

    pub fn set_text_color(&self, color: Option<Rgb>) {
        self.set_exclusive(&[COLOR_PREFIX], color.map(color_tag_name));
    }

    pub fn set_highlight(&self, color: Option<Rgb>) {
        self.set_exclusive(&[HIGHLIGHT_PREFIX], color.map(highlight_tag_name));
    }

    pub fn clear_formatting(&self) {
        let b = self.buffer();
        if let Some(segs) = self.selection_segments() {
            let all = tags_matching(&b, is_char_tag);
            b.begin_user_action();
            for (s, e) in segs {
                remove_tags_range(&b, &all, s, e);
            }
            b.end_user_action();
            self.tags_changed();
        } else {
            let imp = self.imp();
            *imp.pending.borrow_mut() = Some(Vec::new());
            imp.pending_pos.set(cursor_offset(&b));
            self.notify_format();
        }
    }

    // ----- paragraph formatting -----

    fn set_para_family(&self, pred: fn(&str) -> bool, new: Option<&str>) {
        let b = self.buffer();
        let (l0, l1) = selected_lines(&b);
        let old = tags_matching(&b, pred);
        b.begin_user_action();
        for l in l0..=l1 {
            let (s, e) = line_bounds(&b, l);
            remove_tags_range(&b, &old, s, e);
            if let Some(n) = new {
                apply_named(&b, n, true, s, e);
            }
        }
        b.end_user_action();
        self.tags_changed();
    }

    pub fn set_paragraph_style(&self, style: ParagraphStyle) {
        self.set_para_family(|n| n.starts_with("para-"), paragraph_tag(style));
    }

    pub fn set_alignment(&self, a: Alignment) {
        self.set_para_family(|n| n.starts_with("align-"), alignment_tag(a));
    }

    pub fn toggle_list(&self, kind: ListKind) {
        let b = self.buffer();
        let (l0, l1) = selected_lines(&b);
        let all = (l0..=l1).all(|l| list_kind(&b, l) == Some(kind));
        let imp = self.imp();
        b.begin_user_action();
        imp.internal.set(true);
        for l in l0..=l1 {
            if all {
                clear_list_line(&b, l);
            } else if list_kind(&b, l) != Some(kind) {
                set_list_line(&b, l, kind);
            }
        }
        renumber(&b);
        imp.internal.set(false);
        b.end_user_action();
        self.tags_changed();
    }

    // ----- state -----

    pub fn format_state(&self) -> FormatState {
        let b = self.buffer();
        let mut st = FormatState::default();

        let ins = b.iter_at_mark(&b.get_insert());
        let ls = line_start(&b, ins.line());
        for n in tag_names_at(&ls) {
            if let Some(s) = paragraph_style_from_tag(&n) {
                st.paragraph_style = s;
            }
            if let Some(a) = alignment_from_tag(&n) {
                st.alignment = a;
            }
            if let Some(k) = list_from_tag(&n) {
                st.list = Some(k);
            }
        }

        let (names, segs): (Vec<String>, Option<Vec<(i32, i32)>>) = match self.selection_segments() {
            Some(segs) => {
                let first = b.iter_at_offset(segs[0].0);
                (tag_names_at(&first).into_iter().filter(|n| is_char_tag(n)).collect(), Some(segs))
            }
            None => {
                let c = ins.offset();
                let names = match self.imp().pending.borrow().as_ref() {
                    Some(p) => p.clone(),
                    None => inherit_char_tags(&b, c, c),
                };
                (names, None)
            }
        };
        let has = |n: &str| match &segs {
            Some(s) => fully_tagged(&b, n, s),
            None => names.iter().any(|x| x == n),
        };
        st.bold = has(BOLD);
        st.italic = has(ITALIC);
        st.underline = has(UNDERLINE);
        st.strikethrough = has(STRIKETHROUGH);
        st.vertical = if has(SUPERSCRIPT) {
            VerticalAlign::Superscript
        } else if has(SUBSCRIPT) {
            VerticalAlign::Subscript
        } else {
            VerticalAlign::Baseline
        };
        let find = |prefix: &str| names.iter().find(|n| n.starts_with(prefix) && has(n)).map(|n| n[prefix.len()..].to_string());
        st.font_family = find(FONT_PREFIX);
        st.font_size = find(SIZE_PREFIX).and_then(|s| s.parse().ok());
        st.color = find(COLOR_PREFIX).and_then(|s| Rgb::from_hex(&s));
        st.highlight = find(HIGHLIGHT_PREFIX).and_then(|s| Rgb::from_hex(&s));
        st
    }

    /// (words, characters) of the whole document (list markers excluded).
    pub fn counts(&self) -> (usize, usize) {
        let b = self.buffer();
        let (mut words, mut chars) = (0, 0);
        for l in 0..b.line_count() {
            let me = marker_end(&b, l);
            let end = line_end_no_nl(&b, l);
            if end <= me {
                continue;
            }
            let text = b.text(&b.iter_at_offset(me), &b.iter_at_offset(end), false);
            words += text.split_whitespace().count();
            chars += text.chars().count();
        }
        (words, chars)
    }

    /// Enables/disables the typing and caret animations at runtime.
    pub fn set_animations(&self, typing: bool, caret: bool) {
        let imp = self.imp();
        imp.typing.set_enabled(typing);
        imp.caret.set_enabled(caret);
        imp.caret_anim.set(caret);
        if caret {
            self.add_css_class("custom-caret");
        } else {
            self.remove_css_class("custom-caret");
        }
    }

    /// Default font for unformatted text, e.g. ("Cantarell", 12.0).
    #[allow(deprecated)]
    pub fn set_default_font(&self, family: &str, points: f64) {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let imp = self.imp();
        let class = imp
            .css_class
            .borrow_mut()
            .get_or_insert_with(|| format!("gw-editor-{}", COUNTER.fetch_add(1, Ordering::Relaxed)))
            .clone();
        self.add_css_class(&class);
        let family = family.replace('"', "");
        let css = format!(
            "textview.{class}, textview.{class} text {{ font-family: \"{family}\"; font-size: {points}pt; }}"
        );
        let mut slot = imp.provider.borrow_mut();
        if slot.is_none() {
            let p = gtk::CssProvider::new();
            gtk::style_context_add_provider_for_display(
                &self.display(),
                &p,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
            *slot = Some(p);
        }
        slot.as_ref().unwrap().load_from_string(&css);
    }

    /// Emitted whenever `format_state()` may have changed (cursor moved,
    /// selection changed, formatting applied, content changed).
    pub fn connect_format_changed<F: Fn(&Self) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_local("format-changed", false, move |args| {
            let obj = args[0].get::<Self>().unwrap();
            f(&obj);
            None
        })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn select(b: &gtk::TextBuffer, s: i32, e: i32) {
        b.select_range(&b.iter_at_offset(s), &b.iter_at_offset(e));
    }

    // Run from crate::gtk_tests (GTK is single-threaded).
    pub(crate) fn editor_behaviour() {
        if gtk::init().is_err() {
            return;
        }
        let ed = Editor::new();
        let b = ed.buffer();
        b.set_text("hello world\nsecond");

        // toggles
        select(&b, 0, 5);
        ed.toggle_bold();
        assert!(ed.format_state().bold);
        select(&b, 0, 8);
        assert!(!ed.format_state().bold);
        select(&b, 0, 5);
        ed.toggle_bold();
        assert!(!ed.format_state().bold);
        ed.toggle_bold();
        ed.set_font_size(Some(14.0));
        ed.set_font_size(Some(16.0));
        let st = ed.format_state();
        assert_eq!(st.font_size, Some(16.0));
        assert!(st.bold);

        // typing inheritance
        b.place_cursor(&b.iter_at_offset(5));
        b.insert_at_cursor("X");
        assert!(b.iter_at_offset(5).has_tag(&b.tag_table().lookup(BOLD).unwrap()));

        // pending format
        b.place_cursor(&b.iter_at_offset(b.char_count()));
        ed.toggle_italic();
        assert!(ed.format_state().italic);
        b.insert_at_cursor("Z");
        assert!(b.iter_at_offset(b.char_count() - 1).has_tag(&b.tag_table().lookup(ITALIC).unwrap()));
        ed.set_animations(true, false);
        assert!(!ed.has_css_class("custom-caret"));

        // paragraph + lists
        b.set_text("one\ntwo");
        select(&b, 0, 7);
        ed.toggle_list(ListKind::Numbered);
        assert_eq!(b.text(&b.start_iter(), &b.end_iter(), false), "1. one\n2. two");
        assert_eq!(ed.format_state().list, Some(ListKind::Numbered));
        assert_eq!(ed.counts(), (2, 6));
        ed.toggle_list(ListKind::Bullet);
        assert_eq!(b.text(&b.start_iter(), &b.end_iter(), false), "• one\n• two");
        ed.toggle_list(ListKind::Bullet);
        assert_eq!(b.text(&b.start_iter(), &b.end_iter(), false), "one\ntwo");
        ed.set_paragraph_style(ParagraphStyle::Heading1);
        assert_eq!(ed.format_state().paragraph_style, ParagraphStyle::Heading1);
        ed.set_alignment(Alignment::Center);
        assert_eq!(ed.format_state().alignment, Alignment::Center);
    }
}
