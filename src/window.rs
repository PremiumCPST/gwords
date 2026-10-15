//! Main application window: header bar, tab bar, format bar, tab view,
//! search bar, status bar.

use std::cell::{Cell, OnceCell};
use std::path::Path;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib::{self, clone};
use gtk::{gdk, gio};

use crate::document::{Alignment, ListKind, ParagraphStyle};
use crate::document_page::DocumentPage;
use crate::editor::Editor;
use crate::formats::FileFormat;
use crate::settings::{self, Settings};
use crate::ui::advanced_bar::AdvancedBar;
use crate::ui::format_bar::FormatBar;
use crate::ui::search_bar::SearchBar;
use crate::ui::status_bar::StatusBar;

const DEFAULT_NAME: &str = "Document sans titre.odt";

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Window {
        pub tab_view: OnceCell<adw::TabView>,
        pub window_title: OnceCell<adw::WindowTitle>,
        pub toast_overlay: OnceCell<adw::ToastOverlay>,
        pub format_bar: OnceCell<FormatBar>,
        pub advanced_bar: OnceCell<AdvancedBar>,
        pub advanced_revealer: OnceCell<gtk::Revealer>,
        pub search_bar: OnceCell<SearchBar>,
        pub status_bar: OnceCell<StatusBar>,
        pub force_close: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Window {
        const NAME: &'static str = "GwWindow";
        type Type = super::Window;
        type ParentType = adw::ApplicationWindow;
    }

    impl ObjectImpl for Window {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().build_ui();
        }
    }
    impl WidgetImpl for Window {}

    impl WindowImpl for Window {
        fn close_request(&self) -> glib::Propagation {
            let win = self.obj();
            if self.force_close.get() {
                return self.parent_close_request();
            }
            // Persist the window size.
            if !win.is_maximized() && !win.is_fullscreen() {
                let (w, h) = (win.width(), win.height());
                if w > 0 && h > 0 {
                    let mut s = settings::get();
                    if s.window_width != w || s.window_height != h {
                        s.window_width = w;
                        s.window_height = h;
                        settings::set(s);
                    }
                }
            }
            let modified = win.modified_pages();
            if modified.is_empty() {
                return self.parent_close_request();
            }
            let body = if modified.len() == 1 {
                format!(
                    "Les modifications apportées à « {} » seront perdues si vous ne les enregistrez pas.",
                    modified[0].title()
                )
            } else {
                format!(
                    "{} documents contiennent des modifications qui seront perdues si vous ne les enregistrez pas.",
                    modified.len()
                )
            };
            let dialog = confirm_dialog(&body, "Tout enregistrer");
            glib::spawn_future_local(clone!(
                #[weak]
                win,
                async move {
                    match dialog.choose_future(Some(&win)).await.as_str() {
                        "save" => {
                            for page in win.modified_pages() {
                                if !win.save_page(&page).await {
                                    return;
                                }
                            }
                            win.imp().force_close.set(true);
                            win.close();
                        }
                        "discard" => {
                            win.imp().force_close.set(true);
                            win.close();
                        }
                        _ => {}
                    }
                }
            ));
            glib::Propagation::Stop
        }
    }
    impl ApplicationWindowImpl for Window {}
    impl AdwApplicationWindowImpl for Window {}
}

glib::wrapper! {
    pub struct Window(ObjectSubclass<imp::Window>)
        @extends adw::ApplicationWindow, gtk::ApplicationWindow, gtk::Window, gtk::Widget,
        @implements gtk::gio::ActionGroup, gtk::gio::ActionMap, gtk::Accessible, gtk::Buildable,
                    gtk::ConstraintTarget, gtk::Native, gtk::Root, gtk::ShortcutManager;
}

fn confirm_dialog(body: &str, save_label: &str) -> adw::AlertDialog {
    let dialog = adw::AlertDialog::new(Some("Enregistrer les modifications ?"), Some(body));
    dialog.add_responses(&[
        ("cancel", "Annuler"),
        ("discard", "Ne pas enregistrer"),
        ("save", save_label),
    ]);
    dialog.set_response_appearance("discard", adw::ResponseAppearance::Destructive);
    dialog.set_response_appearance("save", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("save"));
    dialog.set_close_response("cancel");
    dialog
}

fn file_filters() -> gio::ListStore {
    let all = gtk::FileFilter::new();
    all.set_name(Some("Tous les documents pris en charge"));
    let odt = gtk::FileFilter::new();
    odt.set_name(Some("Document OpenDocument (.odt)"));
    let docx = gtk::FileFilter::new();
    docx.set_name(Some("Document Word (.docx)"));
    let txt = gtk::FileFilter::new();
    txt.set_name(Some("Texte brut (.txt)"));
    for (f, fmt) in [(&odt, FileFormat::Odt), (&docx, FileFormat::Docx), (&txt, FileFormat::PlainText)] {
        for target in [f, &all] {
            target.add_suffix(fmt.extension());
            target.add_mime_type(fmt.mime_type());
        }
    }
    let store = gio::ListStore::new::<gtk::FileFilter>();
    for f in [all, odt, docx, txt] {
        store.append(&f);
    }
    store
}

fn display_name(file: &gio::File) -> String {
    file.query_info("standard::display-name", gio::FileQueryInfoFlags::NONE, gio::Cancellable::NONE)
        .map(|i| i.display_name().to_string())
        .ok()
        .or_else(|| file.basename().map(|b| b.to_string_lossy().into_owned()))
        .unwrap_or_default()
}

/// Folder shown as subtitle, with the home directory abbreviated to "~".
fn folder_label(file: &gio::File) -> Option<String> {
    let path = file.path()?;
    let parent = path.parent()?;
    Some(abbreviate_home(parent, &glib::home_dir()))
}

fn abbreviate_home(dir: &Path, home: &Path) -> String {
    match dir.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => dir.display().to_string(),
    }
}

fn has_known_extension(name: &str) -> bool {
    matches!(
        Path::new(name).extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref(),
        Some("odt" | "docx" | "txt")
    )
}

impl Window {
    pub fn new(app: &crate::application::Application) -> Self {
        glib::Object::builder().property("application", app).build()
    }

    fn tab_view(&self) -> &adw::TabView {
        self.imp().tab_view.get().expect("tab view")
    }

    fn toast(&self, msg: &str) {
        if let Some(o) = self.imp().toast_overlay.get() {
            o.add_toast(adw::Toast::new(msg));
        }
    }

    fn page_of(tab: &adw::TabPage) -> Option<DocumentPage> {
        tab.child().downcast::<DocumentPage>().ok()
    }

    fn active_page(&self) -> Option<DocumentPage> {
        self.tab_view().selected_page().and_then(|t| Self::page_of(&t))
    }

    fn modified_pages(&self) -> Vec<DocumentPage> {
        let tv = self.tab_view();
        (0..tv.n_pages())
            .filter_map(|i| Self::page_of(&tv.nth_page(i)))
            .filter(|p| p.is_modified())
            .collect()
    }

    fn with_editor(&self, f: impl FnOnce(&Editor)) {
        if let Some(p) = self.active_page() {
            f(&p.editor());
        }
    }

    // ---------------------------------------------------------------- UI

    fn build_ui(&self) {
        let imp = self.imp();
        let s = settings::get();
        self.set_default_size(s.window_width, s.window_height);
        self.set_title(Some("Gwords"));

        let tab_view = adw::TabView::new();
        tab_view.set_vexpand(true);
        let window_title = adw::WindowTitle::new("Gwords", "");
        let format_bar = FormatBar::new();
        let search_bar = SearchBar::new();
        let advanced_bar = AdvancedBar::new();
        let advanced_revealer = gtk::Revealer::builder()
            .transition_type(gtk::RevealerTransitionType::SlideDown)
            .transition_duration(250)
            .reveal_child(s.advanced_mode)
            .child(&advanced_bar)
            .build();
        let status_bar = StatusBar::new();
        status_bar.set_visible(s.show_status_bar);
        let toast_overlay = adw::ToastOverlay::new();

        // Header bar.
        let header = adw::HeaderBar::new();
        header.set_title_widget(Some(&window_title));
        let open_btn = gtk::Button::builder()
            .icon_name("document-open-symbolic")
            .tooltip_text("Ouvrir un document (Ctrl+O)")
            .action_name("win.open")
            .build();
        let tab_btn = gtk::Button::builder()
            .icon_name("tab-new-symbolic")
            .tooltip_text("Nouvel onglet (Ctrl+T)")
            .action_name("win.new-tab")
            .build();
        header.pack_start(&open_btn);
        header.pack_start(&tab_btn);

        let menu = gio::Menu::new();
        let s0 = gio::Menu::new();
        s0.append(Some("Mode avancé"), Some("win.advanced-mode"));
        let s1 = gio::Menu::new();
        s1.append(Some("Nouvel onglet"), Some("win.new-tab"));
        s1.append(Some("Nouvelle fenêtre"), Some("app.new-window"));
        let s2 = gio::Menu::new();
        s2.append(Some("Enregistrer"), Some("win.save"));
        s2.append(Some("Enregistrer sous…"), Some("win.save-as"));
        let s3 = gio::Menu::new();
        s3.append(Some("Rechercher"), Some("win.find"));
        s3.append(Some("Rechercher et remplacer"), Some("win.replace"));
        s3.append(Some("Statistiques du document"), Some("win.statistics"));
        let s4 = gio::Menu::new();
        s4.append(Some("Préférences"), Some("app.preferences"));
        s4.append(Some("À propos de Gwords"), Some("app.about"));
        for sec in [&s0, &s1, &s2, &s3, &s4] {
            menu.append_section(None, sec);
        }
        let menu_btn = gtk::MenuButton::builder()
            .icon_name("open-menu-symbolic")
            .primary(true)
            .tooltip_text("Menu principal")
            .menu_model(&menu)
            .build();
        header.pack_end(&menu_btn);
        let save_btn = gtk::Button::builder()
            .icon_name("document-save-symbolic")
            .tooltip_text("Enregistrer (Ctrl+S)")
            .action_name("win.save")
            .build();
        header.pack_end(&save_btn);

        let tab_bar = adw::TabBar::new();
        tab_bar.set_view(Some(&tab_view));
        tab_bar.set_autohide(true);

        let top = gtk::Box::new(gtk::Orientation::Vertical, 0);
        top.append(&header);
        top.append(&tab_bar);
        top.append(&format_bar);
        top.append(&advanced_revealer);

        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.append(&search_bar);
        content.append(&tab_view);
        toast_overlay.set_child(Some(&content));

        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&top);
        toolbar.set_content(Some(&toast_overlay));
        toolbar.add_bottom_bar(&status_bar);
        self.set_content(Some(&toolbar));

        let _ = imp.tab_view.set(tab_view.clone());
        let _ = imp.window_title.set(window_title);
        let _ = imp.toast_overlay.set(toast_overlay);
        let _ = imp.format_bar.set(format_bar);
        let _ = imp.advanced_bar.set(advanced_bar);
        let _ = imp.advanced_revealer.set(advanced_revealer);
        let _ = imp.search_bar.set(search_bar);
        let _ = imp.status_bar.set(status_bar);

        // Tab view signals.
        tab_view.connect_selected_page_notify(clone!(
            #[weak(rename_to = win)]
            self,
            move |_| win.on_page_changed()
        ));
        tab_view.connect_close_page(clone!(
            #[weak(rename_to = win)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |tv, tab| {
                win.handle_close_page(tv, tab);
                glib::Propagation::Stop
            }
        ));
        tab_view.connect_page_detached(clone!(
            #[weak(rename_to = win)]
            self,
            move |tv, _, _| {
                if tv.n_pages() == 0 && !win.imp().force_close.get() {
                    glib::idle_add_local_once(clone!(
                        #[weak]
                        win,
                        move || {
                            if win.tab_view().n_pages() == 0 && !win.imp().force_close.get() {
                                win.new_tab();
                            }
                        }
                    ));
                }
            }
        ));

        // Drag and drop.
        let drop = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
        drop.connect_drop(clone!(
            #[weak(rename_to = win)]
            self,
            #[upgrade_or]
            false,
            move |_, value, _, _| {
                match value.get::<gdk::FileList>() {
                    Ok(list) => {
                        for f in list.files() {
                            win.open_file(f);
                        }
                        true
                    }
                    Err(_) => false,
                }
            }
        ));
        content.add_controller(drop);

        // Settings.
        settings::connect_changed(clone!(
            #[weak(rename_to = win)]
            self,
            move |s| win.apply_settings(s)
        ));

        self.setup_actions();
        self.new_tab();
    }

    fn apply_settings(&self, s: &Settings) {
        if let Some(r) = self.imp().advanced_revealer.get() {
            r.set_reveal_child(s.advanced_mode);
        }
        if let Some(a) = self.lookup_action("advanced-mode") {
            a.change_state(&s.advanced_mode.to_variant());
        }
        if let Some(sb) = self.imp().status_bar.get() {
            sb.set_visible(s.show_status_bar);
        }
        let tv = self.tab_view();
        for i in 0..tv.n_pages() {
            if let Some(p) = Self::page_of(&tv.nth_page(i)) {
                Self::configure_editor(&p.editor(), s);
            }
        }
    }

    fn configure_editor(editor: &Editor, s: &Settings) {
        editor.set_animations(s.typing_animation, s.caret_animation);
        editor.set_default_font(&s.font_family, s.font_size);
    }

    fn setup_actions(&self) {
        fn simple(name: &str, f: impl Fn(&Window) + 'static) -> gio::ActionEntry<Window> {
            gio::ActionEntry::builder(name).activate(move |w: &Window, _, _| f(w)).build()
        }
        let heading = gio::ActionEntry::builder("heading")
            .parameter_type(Some(glib::VariantTy::INT32))
            .activate(|w: &Window, _, param| {
                let level = param.and_then(|p| p.get::<i32>()).unwrap_or(0);
                let style = match level {
                    1 => ParagraphStyle::Heading1,
                    2 => ParagraphStyle::Heading2,
                    3 => ParagraphStyle::Heading3,
                    _ => ParagraphStyle::Normal,
                };
                w.with_editor(|e| e.set_paragraph_style(style));
            })
            .build();
        let advanced = gio::SimpleAction::new_stateful(
            "advanced-mode",
            None,
            &settings::get().advanced_mode.to_variant(),
        );
        advanced.connect_change_state(clone!(
            #[weak(rename_to = win)]
            self,
            move |action, value| {
                let Some(on) = value.and_then(|v| v.get::<bool>()) else { return };
                if action.state().and_then(|v| v.get::<bool>()) == Some(on) {
                    return;
                }
                action.set_state(&on.to_variant());
                let mut s = settings::get();
                if s.advanced_mode != on {
                    s.advanced_mode = on;
                    settings::set(s);
                    if on {
                        win.toast("Mode avancé activé : tableaux, caractères spéciaux et plus");
                    }
                }
                if let Some(r) = win.imp().advanced_revealer.get() {
                    r.set_reveal_child(on);
                }
            }
        ));
        self.add_action(&advanced);
        self.add_action_entries([
            simple("insert-link", |w| w.with_advanced(|b| b.popup_link_editor())),
            simple("special-chars", |w| w.with_advanced(|b| b.popup_special_chars())),
            simple("statistics", |w| {
                if let Some(b) = w.imp().advanced_bar.get() {
                    b.show_statistics();
                }
            }),
            simple("new-tab", |w| {
                w.new_tab();
            }),
            simple("open", |w| w.show_open_dialog()),
            simple("save", |w| w.save_active()),
            simple("save-as", |w| {
                if let Some(p) = w.active_page() {
                    glib::spawn_future_local(clone!(
                        #[weak]
                        w,
                        async move {
                            w.save_page_as(&p).await;
                        }
                    ));
                }
            }),
            simple("close-tab", |w| {
                if let Some(t) = w.tab_view().selected_page() {
                    w.tab_view().close_page(&t);
                }
            }),
            simple("find", |w| {
                if let Some(s) = w.imp().search_bar.get() {
                    s.show(false);
                }
            }),
            simple("replace", |w| {
                if let Some(s) = w.imp().search_bar.get() {
                    s.show(true);
                }
            }),
            simple("bold", |w| w.with_editor(|e| e.toggle_bold())),
            simple("italic", |w| w.with_editor(|e| e.toggle_italic())),
            simple("underline", |w| w.with_editor(|e| e.toggle_underline())),
            simple("strikethrough", |w| w.with_editor(|e| e.toggle_strikethrough())),
            heading,
            simple("align-left", |w| w.with_editor(|e| e.set_alignment(Alignment::Left))),
            simple("align-center", |w| w.with_editor(|e| e.set_alignment(Alignment::Center))),
            simple("align-right", |w| w.with_editor(|e| e.set_alignment(Alignment::Right))),
            simple("align-justify", |w| w.with_editor(|e| e.set_alignment(Alignment::Justify))),
            simple("bullet-list", |w| w.with_editor(|e| e.toggle_list(ListKind::Bullet))),
            simple("numbered-list", |w| w.with_editor(|e| e.toggle_list(ListKind::Numbered))),
            simple("clear-formatting", |w| w.with_editor(|e| e.clear_formatting())),
        ]);
    }

    /// Runs `f` on the advanced bar, enabling advanced mode first if needed.
    fn with_advanced(&self, f: impl Fn(&AdvancedBar) + 'static) {
        if !settings::get().advanced_mode
            && let Some(a) = self.lookup_action("advanced-mode")
        {
            a.change_state(&true.to_variant());
        }
        glib::idle_add_local_once(clone!(
            #[weak(rename_to = win)]
            self,
            move || {
                if let Some(b) = win.imp().advanced_bar.get() {
                    f(b);
                }
            }
        ));
    }

    // -------------------------------------------------------------- tabs

    fn new_tab(&self) -> (DocumentPage, adw::TabPage) {
        let page = DocumentPage::new();
        Self::configure_editor(&page.editor(), &settings::get());
        let tab = self.tab_view().append(&page);
        Self::sync_tab(&tab, &page);
        page.connect_notify_local(
            None,
            clone!(
                #[weak(rename_to = win)]
                self,
                #[weak]
                tab,
                move |page, pspec| {
                    if matches!(pspec.name(), "title" | "modified") {
                        Self::sync_tab(&tab, page);
                        if win.tab_view().selected_page().as_ref() == Some(&tab) {
                            win.update_title();
                        }
                    }
                }
            ),
        );
        self.tab_view().set_selected_page(&tab);
        (page, tab)
    }

    fn sync_tab(tab: &adw::TabPage, page: &DocumentPage) {
        let title = page.title();
        let shown = if page.is_modified() { format!("• {title}") } else { title.clone() };
        tab.set_title(&shown);
        let tooltip = page
            .file()
            .and_then(|f| f.path())
            .map(|p| p.display().to_string())
            .unwrap_or(title);
        tab.set_tooltip(&tooltip);
    }

    fn on_page_changed(&self) {
        let imp = self.imp();
        let editor = self.active_page().map(|p| p.editor());
        if let Some(b) = imp.format_bar.get() {
            b.set_editor(editor.as_ref());
        }
        if let Some(b) = imp.advanced_bar.get() {
            b.set_editor(editor.as_ref());
        }
        if let Some(b) = imp.search_bar.get() {
            b.set_editor(editor.as_ref());
        }
        if let Some(b) = imp.status_bar.get() {
            b.set_editor(editor.as_ref());
        }
        self.update_title();
        if let Some(e) = editor {
            e.grab_focus();
        }
    }

    fn update_title(&self) {
        let Some(wt) = self.imp().window_title.get() else { return };
        match self.active_page() {
            Some(p) => {
                let title = p.title();
                wt.set_title(&if p.is_modified() { format!("• {title}") } else { title });
                let sub = p.file().and_then(|f| folder_label(&f)).unwrap_or_else(|| "Brouillon".into());
                wt.set_subtitle(&sub);
            }
            None => {
                wt.set_title("Gwords");
                wt.set_subtitle("");
            }
        }
    }

    fn handle_close_page(&self, tv: &adw::TabView, tab: &adw::TabPage) {
        let Some(page) = Self::page_of(tab) else {
            tv.close_page_finish(tab, true);
            return;
        };
        if !page.is_modified() {
            tv.close_page_finish(tab, true);
            return;
        }
        let body = format!(
            "Les modifications apportées à « {} » seront perdues si vous ne les enregistrez pas.",
            page.title()
        );
        let dialog = confirm_dialog(&body, "Enregistrer");
        let (tv, tab) = (tv.clone(), tab.clone());
        glib::spawn_future_local(clone!(
            #[weak(rename_to = win)]
            self,
            async move {
                let confirm = match dialog.choose_future(Some(&win)).await.as_str() {
                    "save" => win.save_page(&page).await,
                    "discard" => true,
                    _ => false,
                };
                tv.close_page_finish(&tab, confirm);
            }
        ));
    }

    // ------------------------------------------------------- open / save

    /// Opens `file` in a new tab (or replaces the current tab if it is blank).
    pub fn open_file(&self, file: gio::File) {
        let tv = self.tab_view();
        for i in 0..tv.n_pages() {
            let tab = tv.nth_page(i);
            if let Some(p) = Self::page_of(&tab)
                && p.file().is_some_and(|f| f.equal(&file))
            {
                tv.set_selected_page(&tab);
                return;
            }
        }
        let active = self.active_page().filter(|p| p.is_blank());
        let (page, created) = match active {
            Some(p) => (p, false),
            None => (self.new_tab().0, true),
        };
        glib::spawn_future_local(clone!(
            #[weak(rename_to = win)]
            self,
            async move {
                match page.load(file.clone()).await {
                    Ok(()) => {
                        let tab = win.tab_view().page(&page);
                        Self::sync_tab(&tab, &page);
                        win.update_title();
                    }
                    Err(e) => {
                        win.toast(&format!("Impossible d'ouvrir « {} » : {e}", display_name(&file)));
                        if created && page.is_blank() && win.tab_view().n_pages() > 1 {
                            let tab = win.tab_view().page(&page);
                            win.tab_view().close_page(&tab);
                        }
                    }
                }
            }
        ));
    }

    fn show_open_dialog(&self) {
        let dialog = gtk::FileDialog::builder().title("Ouvrir un document").modal(true).build();
        let filters = file_filters();
        dialog.set_default_filter(filters.item(0).and_downcast_ref::<gtk::FileFilter>());
        dialog.set_filters(Some(&filters));
        glib::spawn_future_local(clone!(
            #[weak(rename_to = win)]
            self,
            async move {
                if let Ok(files) = dialog.open_multiple_future(Some(&win)).await {
                    for obj in files.iter::<glib::Object>().flatten() {
                        if let Ok(f) = obj.downcast::<gio::File>() {
                            win.open_file(f);
                        }
                    }
                }
            }
        ));
    }

    fn save_active(&self) {
        if let Some(p) = self.active_page() {
            glib::spawn_future_local(clone!(
                #[weak(rename_to = win)]
                self,
                async move {
                    win.save_page(&p).await;
                }
            ));
        }
    }

    /// Saves `page` (asking for a file if needed). Returns true on success.
    async fn save_page(&self, page: &DocumentPage) -> bool {
        if page.file().is_none() {
            return self.save_page_as(page).await;
        }
        match page.save().await {
            Ok(()) => {
                self.toast("Document enregistré");
                true
            }
            Err(e) => {
                self.toast(&format!("Impossible d'enregistrer « {} » : {e}", page.title()));
                false
            }
        }
    }

    async fn save_page_as(&self, page: &DocumentPage) -> bool {
        let dialog = gtk::FileDialog::builder().title("Enregistrer sous").modal(true).build();
        let filters = file_filters();
        dialog.set_default_filter(filters.item(0).and_downcast_ref::<gtk::FileFilter>());
        dialog.set_filters(Some(&filters));
        let name = match page.file() {
            Some(f) => display_name(&f),
            None => DEFAULT_NAME.to_string(),
        };
        dialog.set_initial_name(Some(&name));
        if let Some(folder) = page.file().and_then(|f| f.parent()) {
            dialog.set_initial_folder(Some(&folder));
        }
        let Ok(mut file) = dialog.save_future(Some(self)).await else {
            return false;
        };
        let chosen = display_name(&file);
        if !has_known_extension(&chosen)
            && let Some(parent) = file.parent()
        {
            file = parent.child(format!("{chosen}.odt"));
        }
        match page.save_as(file.clone()).await {
            Ok(()) => {
                Self::sync_tab(&self.tab_view().page(page), page);
                self.update_title();
                self.toast("Document enregistré");
                true
            }
            Err(e) => {
                self.toast(&format!("Impossible d'enregistrer « {} » : {e}", display_name(&file)));
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_abbreviation() {
        let home = Path::new("/home/u");
        assert_eq!(abbreviate_home(Path::new("/home/u"), home), "~");
        assert_eq!(abbreviate_home(Path::new("/home/u/Docs"), home), "~/Docs");
        assert_eq!(abbreviate_home(Path::new("/tmp"), home), "/tmp");
    }

    #[test]
    fn extensions() {
        assert!(has_known_extension("a.ODT"));
        assert!(has_known_extension("a.txt"));
        assert!(!has_known_extension("a"));
        assert!(!has_known_extension("a.pdf"));
    }
}
