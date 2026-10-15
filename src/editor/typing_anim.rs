//! "Ink" animation of freshly typed characters.
//!
//! Pure overlay: for every freshly inserted character a cover rectangle in the
//! page background colour is drawn over the glyph. It is wiped away from the
//! bottom upwards while fading out, which makes the glyph appear to settle in
//! like ink.
//! The buffer is never modified (apart from temporary text marks).
//!
//! Coordinates: `snapshot_layer` draws in buffer coordinates, i.e. the same
//! space as `TextView::iter_location`.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::glib;
use gtk::glib::prelude::*;
use gtk::prelude::*;

/// Total duration of one character animation, in microseconds.
const DURATION_US: i64 = 240_000;
/// Longest insertion (in characters) that is animated.
const MAX_CHARS: usize = 64;
/// Maximum number of simultaneously running animations.
const MAX_ANIMS: usize = 200;

/// Cubic ease-out: fast start, gentle settle.
fn ease_out_cubic(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// Linear progress in `0..=1` for the elapsed time.
fn progress(elapsed_us: i64, duration_us: i64) -> f64 {
    if duration_us <= 0 {
        return 1.0;
    }
    (elapsed_us as f64 / duration_us as f64).clamp(0.0, 1.0)
}

/// Cover geometry for eased progress `e`: returns (full-rect alpha, height
/// fraction of the additional top wipe rectangle, its alpha).
fn cover_params(e: f64) -> (f32, f64, f32) {
    let rest = (1.0 - e).clamp(0.0, 1.0);
    (rest as f32, rest, rest as f32)
}

/// The animated range is tracked by two marks so that later edits (deletion,
/// typing elsewhere) never shift the animation onto other characters:
/// `start` has right gravity and `end` left gravity, so text inserted at
/// either boundary stays outside the range and deletions shrink it.
struct Anim {
    start_mark: gtk::TextMark,
    end_mark: gtk::TextMark,
    /// Frame time (µs) of the first frame this animation was seen in.
    start: Option<i64>,
}

#[derive(Default)]
struct State {
    anims: Vec<Anim>,
    enabled: bool,
    suspended: bool,
    view: glib::WeakRef<gtk::TextView>,
    buffer: Option<(glib::WeakRef<gtk::TextBuffer>, glib::SignalHandlerId)>,
    notify: Option<glib::SignalHandlerId>,
    tick: Option<gtk::TickCallbackId>,
}

impl Anim {
    fn delete_marks(&self) {
        for m in [&self.start_mark, &self.end_mark] {
            if let Some(b) = m.buffer() {
                b.delete_mark(m);
            }
        }
    }
}

impl State {
    fn clear_anims(&mut self) {
        for a in self.anims.drain(..) {
            a.delete_marks();
        }
    }
}

#[derive(Clone)]
pub struct TypingAnimator {
    state: Rc<RefCell<State>>,
}

impl Default for TypingAnimator {
    fn default() -> Self {
        Self {
            state: Rc::new(RefCell::new(State {
                enabled: true,
                ..State::default()
            })),
        }
    }
}

impl TypingAnimator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Hooks into the view's buffer signals and frame clock. Called once from
    /// `Editor::constructed`.
    pub fn attach(&self, view: &gtk::TextView) {
        self.state.borrow_mut().view = view.downgrade();
        self.connect_buffer(view);
        let this = self.clone();
        let id = view.connect_buffer_notify(move |v| this.connect_buffer(v));
        self.state.borrow_mut().notify = Some(id);
    }

    /// While false, insertions are not animated (used by `set_document`).
    pub fn set_suspended(&self, suspended: bool) {
        let mut st = self.state.borrow_mut();
        st.suspended = suspended;
        if suspended {
            st.clear_anims();
        }
    }

    pub fn set_enabled(&self, enabled: bool) {
        let mut st = self.state.borrow_mut();
        st.enabled = enabled;
        if !enabled {
            st.clear_anims();
        }
    }

    /// (Re)connects to the view's current buffer.
    fn connect_buffer(&self, view: &gtk::TextView) {
        let buffer = view.buffer();
        {
            let mut st = self.state.borrow_mut();
            if let Some((weak, id)) = st.buffer.take() {
                if let Some(old) = weak.upgrade() {
                    if old == buffer {
                        st.buffer = Some((weak, id));
                        return;
                    }
                    old.disconnect(id);
                }
            }
            st.clear_anims();
        }
        let this = self.clone();
        let id = buffer.connect_closure(
            "insert-text",
            true, // after the default handler: the iter points past the new text
            glib::closure_local!(move |buf: gtk::TextBuffer,
                                       loc: gtk::TextIter,
                                       text: &str,
                                       _len: i32| {
                this.on_inserted(&buf, &loc, text);
            }),
        );
        self.state.borrow_mut().buffer = Some((buffer.downgrade(), id));
    }

    fn on_inserted(&self, buf: &gtk::TextBuffer, end: &gtk::TextIter, text: &str) {
        let count = text.chars().count();
        if count == 0 || count > MAX_CHARS || text.chars().all(char::is_whitespace) {
            return;
        }
        let Some(view) = self.state.borrow().view.upgrade() else {
            return;
        };
        {
            let st = self.state.borrow();
            if !st.enabled || st.suspended {
                return;
            }
        }
        if !view.settings().property::<bool>("gtk-enable-animations") {
            return;
        }
        let start_off = (end.offset() - count as i32).max(0);
        let start = buf.iter_at_offset(start_off);
        let start_mark = gtk::TextMark::new(None, false);
        let end_mark = gtk::TextMark::new(None, true);
        buf.add_mark(&start_mark, &start);
        buf.add_mark(&end_mark, end);
        {
            let mut st = self.state.borrow_mut();
            st.anims.push(Anim {
                start_mark,
                end_mark,
                // Timed from the insertion itself (not from the first frame
                // that draws it): a view that is redrawn late, e.g. a table
                // cell, must never keep a stale cover over its text.
                start: Some(glib::monotonic_time()),
            });
            if st.anims.len() > MAX_ANIMS {
                st.anims.remove(0).delete_marks();
            }
        }
        self.ensure_tick(&view);
        view.queue_draw();
    }

    fn ensure_tick(&self, view: &gtk::TextView) {
        if self.state.borrow().tick.is_some() {
            return;
        }
        let this = self.clone();
        let id = view.add_tick_callback(move |v, _clock| {
            let now = glib::monotonic_time();
            let done = {
                let mut st = this.state.borrow_mut();
                let mut i = 0;
                while i < st.anims.len() {
                    let finished = st.anims[i]
                        .start
                        .is_some_and(|s| now - s >= DURATION_US);
                    if finished {
                        st.anims.remove(i).delete_marks();
                    } else {
                        i += 1;
                    }
                }
                if st.anims.is_empty() {
                    st.tick = None;
                    true
                } else {
                    false
                }
            };
            v.queue_draw();
            if done {
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
        self.state.borrow_mut().tick = Some(id);
    }

    /// Called from `Editor`'s `snapshot_layer(TextViewLayer::AboveText, ..)`.
    pub fn snapshot_above(&self, view: &gtk::TextView, snapshot: &gtk::Snapshot) {
        let mut st = self.state.borrow_mut();
        if st.anims.is_empty() {
            return;
        }
        let now = glib::monotonic_time();
        let buffer = view.buffer();
        let selection = buffer.selection_bounds().map(|(a, b)| (a.offset(), b.offset()));
        let bg = crate::theme::page_background();

        for anim in st.anims.iter_mut() {
            let start = *anim.start.get_or_insert(now);
            let p = progress(now - start, DURATION_US);
            let e = ease_out_cubic(p);
            if p >= 1.0 {
                continue;
            }
            let (cover_a, wipe_frac, wipe_a) = cover_params(e);
            let mut iter = buffer.iter_at_mark(&anim.start_mark);
            let end = buffer.iter_at_mark(&anim.end_mark);
            while iter < end {
                let ch = iter.char();
                let off = iter.offset();
                let rect = view.iter_location(&iter);
                iter.forward_char();
                if ch.is_whitespace() || rect.width() <= 0 {
                    continue;
                }
                let (x, y, w, h) = (
                    rect.x() as f32,
                    rect.y() as f32,
                    rect.width() as f32,
                    rect.height() as f32,
                );
                let selected = selection.is_some_and(|(a, b)| off >= a && off < b);
                let has_bg = {
                    let at = buffer.iter_at_offset(off);
                    at.tags().iter().any(|t| {
                        t.property::<bool>("background-set")
                            || t.property::<bool>("paragraph-background-set")
                    })
                };
                if !selected && !has_bg {
                    let mut c = bg;
                    c.set_alpha(cover_a);
                    snapshot.append_color(&c, &gtk::graphene::Rect::new(x, y, w, h));
                    let wh = h * wipe_frac as f32;
                    if wh > 0.0 {
                        let mut c2 = bg;
                        c2.set_alpha(wipe_a);
                        snapshot.append_color(&c2, &gtk::graphene::Rect::new(x, y, w, wh));
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn easing_bounds_and_monotonic() {
        assert_eq!(ease_out_cubic(0.0), 0.0);
        assert_eq!(ease_out_cubic(1.0), 1.0);
        assert_eq!(ease_out_cubic(2.0), 1.0);
        assert_eq!(ease_out_cubic(-1.0), 0.0);
        let mut prev = 0.0;
        for i in 1..=100 {
            let v = ease_out_cubic(i as f64 / 100.0);
            assert!(v >= prev);
            prev = v;
        }
        assert!(ease_out_cubic(0.5) > 0.5);
    }

    #[test]
    fn progress_clamps() {
        assert_eq!(progress(-5, 100), 0.0);
        assert_eq!(progress(50, 100), 0.5);
        assert_eq!(progress(500, 100), 1.0);
        assert_eq!(progress(1, 0), 1.0);
    }

    #[test]
    fn cover_fades() {
        let (a, f, w) = cover_params(0.0);
        assert_eq!((a, f, w), (1.0, 1.0, 1.0));
        let (a, f, _) = cover_params(1.0);
        assert_eq!((a, f), (0.0, 0.0));
    }
}
