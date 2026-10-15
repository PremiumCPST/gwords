//! Smooth gliding caret + fading ghosts of deleted text.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::glib::{self, ControlFlow, SignalHandlerId, WeakRef};
use gtk::graphene::{Rect, Size};
use gtk::prelude::*;
use gtk::{gdk, gsk, pango};

const CARET_WIDTH: f32 = 2.0;
/// Idle time before the caret starts pulsing.
const BLINK_DELAY_US: i64 = 600_000;
const BLINK_PERIOD_US: i64 = 1_100_000;
/// Stop animating (caret solid) after this much idle time.
const BLINK_STOP_US: i64 = 10_000_000;
/// Spring angular frequency (settles within ~100 ms).
const OMEGA: f32 = 55.0;
const OMEGA_FAST: f32 = 90.0;
const BIG_JUMP: f32 = 600.0;
const GHOST_US: i64 = 280_000;
const GHOST_MAX: usize = 30;
const GHOST_MAX_CHARS: i32 = 40;

// ---------------------------------------------------------------- pure math

/// Exact step of a critically damped spring. Returns (position, velocity).
fn spring_step(pos: f32, vel: f32, target: f32, omega: f32, dt: f32) -> (f32, f32) {
    let d = pos - target;
    let e = (-omega * dt).exp();
    let tmp = vel + omega * d;
    (target + (d + tmp * dt) * e, (vel - omega * tmp * dt) * e)
}

/// Caret opacity (0..=1) after `idle_us` microseconds without activity.
fn blink_opacity(idle_us: i64, blink: bool) -> f32 {
    if !blink || idle_us < BLINK_DELAY_US || idle_us > BLINK_STOP_US {
        return 1.0;
    }
    let t = (idle_us - BLINK_DELAY_US) as f32 / BLINK_PERIOD_US as f32;
    0.5 + 0.5 * (t * std::f32::consts::TAU).cos()
}

fn ease_out_cubic(t: f32) -> f32 {
    let u = 1.0 - t.clamp(0.0, 1.0);
    1.0 - u * u * u
}

// -------------------------------------------------------------------- state

struct Ghost {
    layout: pango::Layout,
    x: f32,
    y: f32,
    start: i64,
}

#[derive(Default)]
struct Spring {
    pos: [f32; 3], // x, y, height
    vel: [f32; 3],
}

#[derive(Default)]
struct State {
    view: Option<WeakRef<gtk::TextView>>,
    buffer: Option<WeakRef<gtk::TextBuffer>>,
    buffer_ids: Vec<SignalHandlerId>,
    enabled: bool,
    suspended: bool,
    focused: bool,
    initialized: bool,
    target: [f32; 3],
    spring: Spring,
    last_activity: i64,
    last_tick: i64,
    tick: Option<gtk::TickCallbackId>,
    ghosts: Vec<Ghost>,
}

#[derive(Clone, Default)]
pub struct CaretAnimator {
    st: Rc<RefCell<State>>,
}

fn now() -> i64 {
    glib::monotonic_time()
}

fn animations_on(view: &gtk::TextView) -> bool {
    view.settings().is_gtk_enable_animations()
}

fn blink_on(view: &gtk::TextView) -> bool {
    let s = view.settings();
    s.is_gtk_cursor_blink() && s.is_gtk_enable_animations()
}

impl CaretAnimator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Hooks into the view's buffer signals and frame clock. Called once from
    /// `Editor::constructed`.
    pub fn attach(&self, view: &gtk::TextView) {
        {
            let mut st = self.st.borrow_mut();
            st.view = Some(view.downgrade());
            st.focused = view.has_focus();
            st.last_activity = now();
        }
        self.connect_buffer(view);

        let this = self.clone();
        view.connect_buffer_notify(move |v| {
            this.connect_buffer(v);
        });

        let focus = gtk::EventControllerFocus::new();
        let this = self.clone();
        focus.connect_enter(move |c| {
            if let Some(w) = c.widget() {
                this.set_focused(&w, true);
            }
        });
        let this = self.clone();
        focus.connect_leave(move |c| {
            if let Some(w) = c.widget() {
                this.set_focused(&w, false);
            }
        });
        view.add_controller(focus);

        let this = self.clone();
        view.connect_editable_notify(move |v| this.activity(v));
    }

    /// While false, insertions are not animated (used by `set_document`).
    pub fn set_suspended(&self, suspended: bool) {
        let view = {
            let mut st = self.st.borrow_mut();
            st.suspended = suspended;
            if suspended {
                st.ghosts.clear();
                st.initialized = false;
            }
            st.view.as_ref().and_then(|v| v.upgrade())
        };
        if let Some(v) = view {
            v.queue_draw();
        }
    }

    pub fn set_enabled(&self, enabled: bool) {
        let view = {
            let mut st = self.st.borrow_mut();
            st.enabled = enabled;
            st.initialized = false;
            if !enabled {
                st.ghosts.clear();
            }
            st.view.as_ref().and_then(|v| v.upgrade())
        };
        if let Some(v) = view {
            self.activity(&v);
        }
    }

    /// Called from `Editor`'s `snapshot_layer(TextViewLayer::AboveText, ..)`.
    pub fn snapshot_above(&self, view: &gtk::TextView, snapshot: &gtk::Snapshot) {
        if !self.st.borrow().enabled {
            return;
        }
        let t = now();
        self.sync_target(view, t);

        let st = self.st.borrow();
        // Ghosts (drawn even if the caret itself is hidden).
        if !st.ghosts.is_empty() {
            let color = view.color();
            for g in &st.ghosts {
                let p = ((t - g.start) as f32 / GHOST_US as f32).clamp(0.0, 1.0);
                let e = ease_out_cubic(p);
                let alpha = 0.6 * (1.0 - e);
                if alpha <= 0.0 {
                    continue;
                }
                let c = gdk::RGBA::new(color.red(), color.green(), color.blue(), color.alpha() * alpha);
                snapshot.save();
                snapshot.translate(&gtk::graphene::Point::new(g.x, g.y - 6.0 * e));
                let s = 1.0 - 0.1 * e;
                snapshot.scale(s, s);
                snapshot.append_layout(&g.layout, &c);
                snapshot.restore();
            }
        }

        if !Self::caret_visible(&st, view) {
            return;
        }
        let [x, y, h] = st.spring.pos;
        let left = x - CARET_WIDTH / 2.0;
        let right = x + CARET_WIDTH / 2.0;
        let top = y;
        let bottom = y + h;

        let mut color = accent_color();
        let op = blink_opacity(t - st.last_activity, blink_on(view));
        color.set_alpha(color.alpha() * op);
        let rect = Rect::new(left, top, right - left, (bottom - top).max(1.0));
        let r = Size::new(CARET_WIDTH / 2.0, CARET_WIDTH / 2.0);
        if color.alpha() < 0.01 {
            return;
        }
        // Filling a rounded path (instead of push_rounded_clip) avoids GSK
        // "everything is clipped" warnings when the caret is scrolled out.
        let rr = gsk::RoundedRect::new(rect, r, r, r, r);
        let builder = gsk::PathBuilder::new();
        builder.add_rounded_rect(&rr);
        snapshot.append_fill(&builder.to_path(), gsk::FillRule::Winding, &color);
    }

    // ---------------------------------------------------------- internals

    fn caret_visible(st: &State, view: &gtk::TextView) -> bool {
        st.enabled
            && st.focused
            && st.initialized
            && view.is_editable()
            && view
                .buffer()
                .selection_bounds()
                .is_none()
    }

    fn set_focused(&self, view: &gtk::Widget, focused: bool) {
        {
            let mut st = self.st.borrow_mut();
            st.focused = focused;
            st.initialized = false;
            st.last_activity = now();
        }
        view.queue_draw();
        if let Ok(v) = view.clone().downcast::<gtk::TextView>() {
            self.activity(&v);
        }
    }

    /// Reset the blink phase and make sure frames are produced.
    fn activity(&self, view: &gtk::TextView) {
        self.st.borrow_mut().last_activity = now();
        self.ensure_tick(view);
        view.queue_draw();
    }

    fn connect_buffer(&self, view: &gtk::TextView) {
        let buffer = view.buffer();
        {
            let mut st = self.st.borrow_mut();
            if let Some(old) = st.buffer.as_ref().and_then(|b| b.upgrade()) {
                for id in st.buffer_ids.drain(..) {
                    old.disconnect(id);
                }
            }
            st.buffer_ids.clear();
            st.buffer = Some(buffer.downgrade());
            st.initialized = false;
        }
        let mut ids = Vec::new();

        let this = self.clone();
        let vw = view.downgrade();
        ids.push(buffer.connect_mark_set(move |b, _, mark| {
            if mark == &b.get_insert() {
                if let Some(v) = vw.upgrade() {
                    this.activity(&v);
                }
            }
        }));

        let this = self.clone();
        let vw = view.downgrade();
        ids.push(buffer.connect_changed(move |_| {
            if let Some(v) = vw.upgrade() {
                this.activity(&v);
            }
        }));

        let this = self.clone();
        let vw = view.downgrade();
        ids.push(buffer.connect_delete_range(move |_, start, end| {
            if let Some(v) = vw.upgrade() {
                this.spawn_ghost(&v, start, end);
            }
        }));

        self.st.borrow_mut().buffer_ids = ids;
        self.activity(view);
    }

    fn spawn_ghost(&self, view: &gtk::TextView, start: &gtk::TextIter, end: &gtk::TextIter) {
        {
            let st = self.st.borrow();
            if !st.enabled || st.suspended || !animations_on(view) || !view.is_mapped() {
                return;
            }
        }
        if end.offset() - start.offset() > GHOST_MAX_CHARS {
            return;
        }
        let text = start.text(end);
        if text.is_empty() || text.contains('\n') {
            return;
        }
        let layout = view.create_pango_layout(Some(text.as_str()));
        apply_tag_attrs(&layout, start);
        let rect = view.iter_location(start);
        let mut st = self.st.borrow_mut();
        if st.ghosts.len() >= GHOST_MAX {
            st.ghosts.remove(0);
        }
        st.ghosts.push(Ghost {
            layout,
            x: rect.x() as f32,
            y: rect.y() as f32,
            start: now(),
        });
        drop(st);
        self.ensure_tick(view);
        view.queue_draw();
    }

    /// Reads the current cursor rect and retargets the spring if it changed.
    fn sync_target(&self, view: &gtk::TextView, t: i64) {
        let loc = view.cursor_locations(None).0;
        let target = [loc.x() as f32, loc.y() as f32, loc.height().max(1) as f32];
        let animate = animations_on(view);
        let mut start_tick = false;
        {
            let mut st = self.st.borrow_mut();
            if !st.initialized || st.suspended || !animate {
                st.initialized = true;
                st.target = target;
                st.spring = Spring { pos: target, vel: [0.0; 3] };
            } else if st.target != target {
                st.target = target;
                st.last_activity = t;
                start_tick = true;
            }
        }
        if start_tick {
            self.ensure_tick(view);
        }
    }

    fn animating(st: &State, view: &gtk::TextView, t: i64) -> bool {
        let moving = (0..3).any(|i| {
            (st.spring.pos[i] - st.target[i]).abs() > 0.05 || st.spring.vel[i].abs() > 0.5
        });
        let visible = st.enabled && st.focused && view.is_editable();
        let blinking = visible
            && blink_on(view)
            && t - st.last_activity <= BLINK_STOP_US;
        st.enabled && (moving || !st.ghosts.is_empty() || blinking)
    }

    fn ensure_tick(&self, view: &gtk::TextView) {
        if self.st.borrow().tick.is_some() || !self.st.borrow().enabled {
            return;
        }
        self.st.borrow_mut().last_tick = now();
        let this = self.clone();
        let id = view.add_tick_callback(move |w, _clock| this.on_tick(w));
        self.st.borrow_mut().tick = Some(id);
    }

    fn on_tick(&self, widget: &gtk::TextView) -> ControlFlow {
        let t = now();
        {
            let mut st = self.st.borrow_mut();
            let dt = ((t - st.last_tick) as f32 / 1e6).clamp(0.0, 0.05);
            st.last_tick = t;
            st.ghosts.retain(|g| t - g.start < GHOST_US);
            let dist = {
                let dx = st.spring.pos[0] - st.target[0];
                let dy = st.spring.pos[1] - st.target[1];
                (dx * dx + dy * dy).sqrt()
            };
            let omega = if dist > BIG_JUMP { OMEGA_FAST } else { OMEGA };
            for i in 0..3 {
                let (p, v) =
                    spring_step(st.spring.pos[i], st.spring.vel[i], st.target[i], omega, dt);
                st.spring.pos[i] = p;
                st.spring.vel[i] = v;
            }
            let settled = (0..3).all(|i| {
                (st.spring.pos[i] - st.target[i]).abs() <= 0.05 && st.spring.vel[i].abs() <= 0.5
            });
            if settled {
                st.spring = Spring { pos: st.target, vel: [0.0; 3] };
            }
        }
        widget.queue_draw();
        let keep = Self::animating(&self.st.borrow(), widget, t);
        if !keep {
            self.st.borrow_mut().tick = None;
            return ControlFlow::Break;
        }
        ControlFlow::Continue
    }
}

fn accent_color() -> gdk::RGBA {
    adw::StyleManager::default().accent_color_rgba()
}

/// Approximate the deleted text's look using the tags at `iter`.
fn apply_tag_attrs(layout: &pango::Layout, iter: &gtk::TextIter) {
    let attrs = pango::AttrList::new();
    for tag in iter.tags() {
        let weight: i32 = tag.property("weight");
        if weight > 400 {
            attrs.insert(pango::AttrInt::new_weight(pango::Weight::__Unknown(weight)));
        }
        let style: pango::Style = tag.property("style");
        if style != pango::Style::Normal {
            attrs.insert(pango::AttrInt::new_style(style));
        }
        let size: i32 = tag.property("size");
        if size > 0 {
            attrs.insert(pango::AttrSize::new(size));
        }
    }
    layout.set_attributes(Some(&attrs));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spring_converges_without_overshoot() {
        let (mut p, mut v) = (0.0f32, 0.0f32);
        let mut t = 0.0;
        while t < 0.1 {
            (p, v) = spring_step(p, v, 100.0, OMEGA, 1.0 / 60.0);
            assert!(p <= 100.0 + 1e-3);
            t += 1.0 / 60.0;
        }
        assert!(p > 90.0, "p={p}");
        for _ in 0..60 {
            (p, v) = spring_step(p, v, 100.0, OMEGA, 1.0 / 60.0);
        }
        assert!((p - 100.0).abs() < 0.01 && v.abs() < 0.5);
    }

    #[test]
    fn spring_is_step_size_independent() {
        let a = spring_step(0.0, 0.0, 10.0, OMEGA, 0.02);
        let (p, v) = spring_step(0.0, 0.0, 10.0, OMEGA, 0.01);
        let b = spring_step(p, v, 10.0, OMEGA, 0.01);
        assert!((a.0 - b.0).abs() < 1e-3 && (a.1 - b.1).abs() < 1e-2);
    }

    #[test]
    fn blink_math() {
        assert_eq!(blink_opacity(0, true), 1.0);
        assert_eq!(blink_opacity(500_000, true), 1.0);
        assert_eq!(blink_opacity(2_000_000, false), 1.0);
        assert_eq!(blink_opacity(BLINK_STOP_US + 1, true), 1.0);
        assert!((blink_opacity(BLINK_DELAY_US, true) - 1.0).abs() < 1e-5);
        let half = blink_opacity(BLINK_DELAY_US + BLINK_PERIOD_US / 2, true);
        assert!(half < 0.01);
        for i in 0..200 {
            let o = blink_opacity(BLINK_DELAY_US + i * 40_000, true);
            assert!((0.0..=1.0).contains(&o));
        }
    }

    #[test]
    fn easing_bounds() {
        assert_eq!(ease_out_cubic(0.0), 0.0);
        assert_eq!(ease_out_cubic(1.0), 1.0);
        assert!(ease_out_cubic(0.5) > 0.5);
    }
}
