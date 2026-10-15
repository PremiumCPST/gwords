//! Shared visual constants. `data/style.css` must stay consistent with these.

/// Background of the "paper" page in light mode (matches CSS `.page` in light mode).
pub const PAGE_BG_LIGHT: (f32, f32, f32) = (1.0, 1.0, 1.0); // #ffffff
/// Background of the "paper" page in dark mode (matches CSS `.page` in dark mode).
pub const PAGE_BG_DARK: (f32, f32, f32) = (0.141, 0.141, 0.157); // #242428

/// Accent-ish color used for the animated caret when the system accent is unavailable.
pub const CARET_FALLBACK: (f32, f32, f32) = (0.208, 0.518, 0.894); // #3584e4

/// Max width (px) of the page column.
pub const PAGE_MAX_WIDTH: i32 = 820;
/// Inner horizontal / vertical padding of the page (TextView margins), px.
pub const PAGE_PADDING_X: i32 = 72;
pub const PAGE_PADDING_Y: i32 = 64;

/// Returns the page background for the current libadwaita color scheme.
pub fn page_background() -> gtk::gdk::RGBA {
    let (r, g, b) = if adw::StyleManager::default().is_dark() { PAGE_BG_DARK } else { PAGE_BG_LIGHT };
    gtk::gdk::RGBA::new(r, g, b, 1.0)
}
