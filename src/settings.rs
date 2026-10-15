//! Persistent user settings stored in `~/.config/gwords/settings.ini`
//! (glib::KeyFile, group "General"). Process-wide singleton, main thread only.

use std::cell::RefCell;
use std::path::PathBuf;

use gtk::glib::{self, KeyFile, KeyFileFlags};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorScheme {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub typing_animation: bool,
    pub caret_animation: bool,
    pub font_family: String,
    pub font_size: f64,
    pub show_status_bar: bool,
    pub color_scheme: ColorScheme,
    pub window_width: i32,
    pub window_height: i32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            typing_animation: true,
            caret_animation: true,
            font_family: "Cantarell".into(),
            font_size: 12.0,
            show_status_bar: true,
            color_scheme: ColorScheme::System,
            window_width: 1000,
            window_height: 760,
        }
    }
}

const GROUP: &str = "General";

struct State {
    current: RefCell<Option<Settings>>,
    listeners: RefCell<Vec<Box<dyn Fn(&Settings)>>>,
}

thread_local! {
    static STATE: State = State {
        current: RefCell::new(None),
        listeners: RefCell::new(Vec::new()),
    };
}

/// Current settings (loaded from disk on first call).
pub fn get() -> Settings {
    STATE.with(|st| {
        if let Some(s) = st.current.borrow().as_ref() {
            return s.clone();
        }
        let loaded = load();
        *st.current.borrow_mut() = Some(loaded.clone());
        loaded
    })
}

/// Replaces the settings, saves them to disk, and notifies listeners.
///
/// Listeners are taken out of the state while they run, so they may call `get()`
/// (and `connect_changed`). A `set()` issued from inside a listener stores and saves
/// the new value but does not re-notify the listeners of the outer notification.
pub fn set(settings: Settings) {
    save(&settings);
    STATE.with(|st| *st.current.borrow_mut() = Some(settings.clone()));

    let listeners = STATE.with(|st| std::mem::take(&mut *st.listeners.borrow_mut()));
    for listener in &listeners {
        listener(&settings);
    }
    // Put the original listeners back first, keeping any registered during notification after them.
    STATE.with(|st| {
        let mut current = st.listeners.borrow_mut();
        let added = std::mem::take(&mut *current);
        *current = listeners;
        current.extend(added);
    });
}

/// Registers a listener called (on the main thread) after every `set`.
pub fn connect_changed<F: Fn(&Settings) + 'static>(f: F) {
    STATE.with(|st| st.listeners.borrow_mut().push(Box::new(f)));
}

fn config_path() -> PathBuf {
    glib::user_config_dir().join("gwords").join("settings.ini")
}

fn load() -> Settings {
    let kf = KeyFile::new();
    let path = config_path();
    if path.exists() {
        if let Err(e) = kf.load_from_file(&path, KeyFileFlags::NONE) {
            eprintln!("Préférences illisibles ({}) : {e}", path.display());
        }
    }
    from_keyfile(&kf)
}

fn save(settings: &Settings) {
    let path = config_path();
    if let Some(dir) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(dir) {
            eprintln!("Impossible de créer {} : {e}", dir.display());
            return;
        }
    }
    if let Err(e) = to_keyfile(settings).save_to_file(&path) {
        eprintln!("Impossible d'enregistrer les préférences ({}) : {e}", path.display());
    }
}

fn color_scheme_name(scheme: ColorScheme) -> &'static str {
    match scheme {
        ColorScheme::System => "system",
        ColorScheme::Light => "light",
        ColorScheme::Dark => "dark",
    }
}

fn parse_color_scheme(name: &str) -> Option<ColorScheme> {
    match name.trim().to_ascii_lowercase().as_str() {
        "system" => Some(ColorScheme::System),
        "light" => Some(ColorScheme::Light),
        "dark" => Some(ColorScheme::Dark),
        _ => None,
    }
}

fn to_keyfile(settings: &Settings) -> KeyFile {
    let kf = KeyFile::new();
    kf.set_boolean(GROUP, "typing-animation", settings.typing_animation);
    kf.set_boolean(GROUP, "caret-animation", settings.caret_animation);
    kf.set_string(GROUP, "font-family", &settings.font_family);
    kf.set_double(GROUP, "font-size", settings.font_size);
    kf.set_boolean(GROUP, "show-status-bar", settings.show_status_bar);
    kf.set_string(GROUP, "color-scheme", color_scheme_name(settings.color_scheme));
    kf.set_integer(GROUP, "window-width", settings.window_width);
    kf.set_integer(GROUP, "window-height", settings.window_height);
    kf
}

/// Reads settings from a KeyFile. Missing or invalid keys fall back to the defaults.
fn from_keyfile(kf: &KeyFile) -> Settings {
    let defaults = Settings::default();
    let boolean = |key: &str, default: bool| kf.boolean(GROUP, key).unwrap_or(default);

    let font_family = kf
        .string(GROUP, "font-family")
        .ok()
        .map(|s| s.to_string())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(defaults.font_family);
    let font_size = kf
        .double(GROUP, "font-size")
        .ok()
        .filter(|v| v.is_finite() && *v > 0.0)
        .unwrap_or(defaults.font_size);
    let color_scheme = kf
        .string(GROUP, "color-scheme")
        .ok()
        .and_then(|s| parse_color_scheme(s.as_str()))
        .unwrap_or(defaults.color_scheme);
    let window_width = kf
        .integer(GROUP, "window-width")
        .ok()
        .filter(|v| *v > 0)
        .unwrap_or(defaults.window_width);
    let window_height = kf
        .integer(GROUP, "window-height")
        .ok()
        .filter(|v| *v > 0)
        .unwrap_or(defaults.window_height);

    Settings {
        typing_animation: boolean("typing-animation", defaults.typing_animation),
        caret_animation: boolean("caret-animation", defaults.caret_animation),
        font_family,
        font_size,
        show_status_bar: boolean("show-status-bar", defaults.show_status_bar),
        color_scheme,
        window_width,
        window_height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_scheme_mapping() {
        for scheme in [ColorScheme::System, ColorScheme::Light, ColorScheme::Dark] {
            assert_eq!(parse_color_scheme(color_scheme_name(scheme)), Some(scheme));
        }
        assert_eq!(parse_color_scheme(" DARK "), Some(ColorScheme::Dark));
        assert_eq!(parse_color_scheme("pink"), None);
        assert_eq!(parse_color_scheme(""), None);
    }

    #[test]
    fn keyfile_roundtrip() {
        let settings = Settings {
            typing_animation: false,
            caret_animation: false,
            font_family: "Noto Serif, Italic".into(),
            font_size: 13.5,
            show_status_bar: false,
            color_scheme: ColorScheme::Dark,
            window_width: 1200,
            window_height: 900,
        };
        let data = to_keyfile(&settings).to_data();
        let kf = KeyFile::new();
        kf.load_from_data(data.as_str(), KeyFileFlags::NONE).unwrap();
        assert_eq!(from_keyfile(&kf), settings);
    }

    #[test]
    fn missing_keys_use_defaults() {
        assert_eq!(from_keyfile(&KeyFile::new()), Settings::default());
    }

    #[test]
    fn invalid_values_fall_back_to_defaults() {
        let kf = KeyFile::new();
        kf.load_from_data(
            "[General]\nfont-size=abc\ncolor-scheme=pink\nwindow-width=-4\nfont-family=  \nshow-status-bar=maybe\n",
            KeyFileFlags::NONE,
        )
        .unwrap();
        assert_eq!(from_keyfile(&kf), Settings::default());
    }
}
