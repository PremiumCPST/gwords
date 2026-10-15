//! The `adw::Application` subclass.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;
use gtk::{gdk, gio};

use crate::settings::{self, ColorScheme};
use crate::window::Window;

pub const APP_ID: &str = "io.github.gwords.Gwords";

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Application {}

    #[glib::object_subclass]
    impl ObjectSubclass for Application {
        const NAME: &'static str = "GwApplication";
        type Type = super::Application;
        type ParentType = adw::Application;
    }

    impl ObjectImpl for Application {}

    impl ApplicationImpl for Application {
        fn startup(&self) {
            self.parent_startup();
            let app = self.obj();
            load_css();
            apply_color_scheme(settings::get().color_scheme);
            settings::connect_changed(|s| apply_color_scheme(s.color_scheme));
            gtk::Window::set_default_icon_name(APP_ID);
            app.setup_actions();
        }

        fn activate(&self) {
            self.parent_activate();
            let window = Window::new(&self.obj());
            window.present();
        }

        fn open(&self, files: &[gio::File], _hint: &str) {
            self.parent_open(files, _hint);
            let app = self.obj();
            let window = app.active_window().and_then(|w| w.downcast::<Window>().ok());
            let window = window.unwrap_or_else(|| Window::new(&app));
            for file in files {
                window.open_file(file.clone());
            }
            window.present();
        }
    }

    impl GtkApplicationImpl for Application {}
    impl AdwApplicationImpl for Application {}

    fn load_css() {
        let Some(display) = gdk::Display::default() else { return };
        let provider = gtk::CssProvider::new();
        provider.load_from_string(include_str!("../data/style.css"));
        // App providers don't follow libadwaita's dark mode by themselves:
        // drive `@media (prefers-color-scheme: dark)` from the StyleManager.
        let style_manager = adw::StyleManager::default();
        let sync = |provider: &gtk::CssProvider, sm: &adw::StyleManager| {
            provider.set_prefers_color_scheme(if sm.is_dark() {
                gtk::InterfaceColorScheme::Dark
            } else {
                gtk::InterfaceColorScheme::Light
            });
        };
        sync(&provider, &style_manager);
        style_manager.connect_dark_notify(glib::clone!(
            #[weak]
            provider,
            move |sm| sync(&provider, sm)
        ));
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }

    fn apply_color_scheme(scheme: ColorScheme) {
        let scheme = match scheme {
            ColorScheme::System => adw::ColorScheme::Default,
            ColorScheme::Light => adw::ColorScheme::ForceLight,
            ColorScheme::Dark => adw::ColorScheme::ForceDark,
        };
        adw::StyleManager::default().set_color_scheme(scheme);
    }
}

glib::wrapper! {
    pub struct Application(ObjectSubclass<imp::Application>)
        @extends adw::Application, gtk::Application, gtk::gio::Application,
        @implements gtk::gio::ActionGroup, gtk::gio::ActionMap;
}

impl Default for Application {
    fn default() -> Self {
        Self::new()
    }
}

impl Application {
    pub fn new() -> Self {
        glib::Object::builder()
            .property("application-id", APP_ID)
            .property("flags", gtk::gio::ApplicationFlags::HANDLES_OPEN)
            .build()
    }

    fn setup_actions(&self) {
        let entries = [
            gio::ActionEntry::builder("quit")
                .activate(|app: &Self, _, _| {
                    for w in app.windows() {
                        w.close();
                    }
                })
                .build(),
            gio::ActionEntry::builder("new-window")
                .activate(|app: &Self, _, _| Window::new(app).present())
                .build(),
            gio::ActionEntry::builder("preferences")
                .activate(|app: &Self, _, _| {
                    let dialog = crate::ui::preferences::preferences_dialog();
                    dialog.present(app.active_window().as_ref());
                })
                .build(),
            gio::ActionEntry::builder("about")
                .activate(|app: &Self, _, _| {
                    let about = adw::AboutDialog::builder()
                        .application_name("Gwords")
                        .application_icon(APP_ID)
                        .developer_name("Gwords")
                        .version(env!("CARGO_PKG_VERSION"))
                        .license_type(gtk::License::Gpl30)
                        .comments("Un traitement de texte simple et élégant pour GNOME.")
                        .build();
                    about.present(app.active_window().as_ref());
                })
                .build(),
        ];
        self.add_action_entries(entries);

        self.set_accels_for_action("app.quit", &["<Control>q"]);
        self.set_accels_for_action("app.new-window", &["<Control><Shift>n"]);
        self.set_accels_for_action("app.preferences", &["<Control>comma"]);

        let win_accels: &[(&str, &[&str])] = &[
            ("win.new-tab", &["<Control>n", "<Control>t"]),
            ("win.open", &["<Control>o"]),
            ("win.save", &["<Control>s"]),
            ("win.save-as", &["<Control><Shift>s"]),
            ("win.close-tab", &["<Control>w"]),
            ("win.find", &["<Control>f"]),
            ("win.replace", &["<Control>h"]),
            ("win.bold", &["<Control>b"]),
            ("win.italic", &["<Control>i"]),
            ("win.underline", &["<Control>u"]),
            ("win.strikethrough", &["<Control><Shift>x"]),
            ("win.heading(0)", &["<Control>0"]),
            ("win.heading(1)", &["<Control>1"]),
            ("win.heading(2)", &["<Control>2"]),
            ("win.heading(3)", &["<Control>3"]),
            ("win.align-left", &["<Control>l"]),
            ("win.align-center", &["<Control>e"]),
            ("win.align-right", &["<Control>r"]),
            ("win.align-justify", &["<Control>j"]),
            ("win.bullet-list", &["<Control><Shift>8"]),
            ("win.numbered-list", &["<Control><Shift>7"]),
            ("win.clear-formatting", &["<Control>m"]),
        ];
        for (action, accels) in win_accels {
            self.set_accels_for_action(action, accels);
        }
    }
}
