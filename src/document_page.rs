//! One open document: the scrolled "paper page" containing an `Editor`,
//! plus its file and modified state. Shown inside an `adw::TabPage`.

use std::cell::{Cell, RefCell};

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gio, glib};

use crate::document::Metadata;
use crate::editor::Editor;
use crate::formats::{self, FileFormat};
use crate::{settings, theme};

const NEW_TITLE: &str = "Nouveau document";
const BOX_MARGIN_TOP: i32 = 32;
const BOX_MARGIN_START_FROM: i32 = 56;

fn format_of(file: &gio::File) -> FileFormat {
    match file.path() {
        Some(p) => FileFormat::from_path(&p),
        None => FileFormat::from_path(std::path::Path::new(
            &file.basename().unwrap_or_default(),
        )),
    }
}

fn file_title(file: &gio::File) -> String {
    file.basename()
        .map(|b| b.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| NEW_TITLE.to_string())
}

mod imp {
    use super::*;

    pub struct DocumentPage {
        pub editor: Editor,
        pub scrolled: gtk::ScrolledWindow,
        pub content: gtk::Box,
        pub clamp: adw::Clamp,
        pub file: RefCell<Option<gio::File>>,
        pub metadata: RefCell<Metadata>,
        pub enter_anim: RefCell<Option<adw::TimedAnimation>>,
        pub scroll_anim: RefCell<Option<adw::TimedAnimation>>,
        pub scroll_pending: Cell<bool>,
        pub entered: Cell<bool>,
    }

    impl Default for DocumentPage {
        fn default() -> Self {
            Self {
                editor: Editor::new(),
                scrolled: gtk::ScrolledWindow::new(),
                content: gtk::Box::new(gtk::Orientation::Vertical, 0),
                clamp: adw::Clamp::new(),
                file: RefCell::default(),
                metadata: RefCell::default(),
                enter_anim: RefCell::default(),
                scroll_anim: RefCell::default(),
                scroll_pending: Cell::new(false),
                entered: Cell::new(false),
            }
        }
    }

    impl DocumentPage {
        fn title_value(&self) -> String {
            match self.file.borrow().as_ref() {
                Some(f) => file_title(f),
                None => NEW_TITLE.to_string(),
            }
        }

        fn modified_value(&self) -> bool {
            self.editor.buffer().is_modified()
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for DocumentPage {
        const NAME: &'static str = "GwDocumentPage";
        type Type = super::DocumentPage;
        type ParentType = adw::Bin;
    }

    impl ObjectImpl for DocumentPage {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPS: std::sync::OnceLock<Vec<glib::ParamSpec>> = std::sync::OnceLock::new();
            PROPS.get_or_init(|| {
                vec![
                    glib::ParamSpecString::builder("title").read_only().build(),
                    glib::ParamSpecBoolean::builder("modified").read_only().build(),
                ]
            })
        }

        fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "title" => self.title_value().to_value(),
                "modified" => self.modified_value().to_value(),
                _ => unreachable!(),
            }
        }

        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();

            self.scrolled.add_css_class("document-backdrop");
            self.scrolled.set_hscrollbar_policy(gtk::PolicyType::Never);
            self.scrolled.set_kinetic_scrolling(true);
            self.scrolled.set_hexpand(true);
            self.scrolled.set_vexpand(true);

            self.content.set_margin_top(BOX_MARGIN_TOP);
            self.content.set_margin_bottom(32);
            self.content.set_margin_start(24);
            self.content.set_margin_end(24);

            self.clamp.set_maximum_size(theme::PAGE_MAX_WIDTH);
            self.clamp.set_tightening_threshold(600);
            self.clamp.set_vexpand(true);

            self.editor.set_vexpand(true);
            let s = settings::get();
            self.editor.set_animations(s.typing_animation, s.caret_animation);
            self.editor.set_default_font(&s.font_family, s.font_size);

            self.clamp.set_child(Some(&self.editor));
            self.content.append(&self.clamp);
            self.scrolled.set_child(Some(&self.content));
            obj.set_child(Some(&self.scrolled));

            // Click on the backdrop: focus the editor, caret at end.
            let click = gtk::GestureClick::new();
            click.connect_released(glib::clone!(
                #[weak]
                obj,
                move |gesture, _, x, y| {
                    let editor = obj.editor();
                    // Only clicks on the backdrop (around/below the page) move
                    // the cursor to the end: clicks inside the page or a table
                    // cell belong to the editors themselves.
                    if let Some(target) = gesture.widget().and_then(|w| w.pick(x, y, gtk::PickFlags::DEFAULT)) {
                        if target == *editor.upcast_ref::<gtk::Widget>() || target.is_ancestor(&editor) {
                            return;
                        }
                    }
                    let buffer = editor.buffer();
                    buffer.place_cursor(&buffer.end_iter());
                    editor.grab_focus();
                }
            ));
            self.content.add_controller(click);

            // Modified state.
            self.editor.buffer().connect_modified_changed(glib::clone!(
                #[weak]
                obj,
                move |_| obj.notify("modified")
            ));

            // Smooth scroll to cursor.
            let buffer = self.editor.buffer();
            buffer.connect_mark_set(glib::clone!(
                #[weak]
                obj,
                move |buf, _, mark| {
                    if mark == &buf.get_insert() {
                        obj.schedule_scroll();
                    }
                }
            ));
            buffer.connect_changed(glib::clone!(
                #[weak]
                obj,
                move |_| obj.schedule_scroll()
            ));

            // Entrance animation on first map.
            obj.connect_map(|obj| {
                let imp = obj.imp();
                if !imp.entered.replace(true) {
                    obj.play_entrance();
                }
            });
        }
    }
    impl WidgetImpl for DocumentPage {}
    impl BinImpl for DocumentPage {}
}

glib::wrapper! {
    /// Properties (notify-able): "title" (String), "modified" (bool).
    pub struct DocumentPage(ObjectSubclass<imp::DocumentPage>)
        @extends adw::Bin, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for DocumentPage {
    fn default() -> Self {
        Self::new()
    }
}

impl DocumentPage {
    /// New empty, unmodified document titled "Nouveau document".
    pub fn new() -> Self {
        glib::Object::new()
    }

    pub fn editor(&self) -> Editor {
        self.imp().editor.clone()
    }

    pub fn file(&self) -> Option<gio::File> {
        self.imp().file.borrow().clone()
    }

    /// File display name, or "Nouveau document".
    pub fn title(&self) -> String {
        self.property("title")
    }

    pub fn is_modified(&self) -> bool {
        self.imp().editor.buffer().is_modified()
    }

    /// True if the document is unmodified, has no file and is empty
    /// (can be replaced when opening a file).
    pub fn is_blank(&self) -> bool {
        self.file().is_none() && !self.is_modified() && self.imp().editor.buffer().char_count() == 0
    }

    fn set_file(&self, file: Option<gio::File>) {
        *self.imp().file.borrow_mut() = file;
        self.notify("title");
    }

    /// Loads `file` (format from extension). Parsing happens off the main thread.
    pub async fn load(&self, file: gio::File) -> Result<(), String> {
        let (bytes, _) = file
            .load_contents_future()
            .await
            .map_err(|e| format!("Le fichier est illisible : {e}"))?;
        let bytes = bytes.to_vec();
        let format = format_of(&file);
        let doc = gio::spawn_blocking(move || formats::read_bytes(&bytes, format))
            .await
            .map_err(|_| "Le fichier est illisible : échec interne".to_string())?
            .map_err(|e| format!("Le fichier est illisible : {e}"))?;

        let imp = self.imp();
        imp.editor.set_document(&doc);
        *imp.metadata.borrow_mut() = doc.metadata.clone();
        imp.editor.buffer().set_modified(false);
        self.set_file(Some(file));
        self.notify("modified");
        imp.entered.set(true);
        self.play_entrance();
        Ok(())
    }

    /// Saves to the current file. Errors if there is no file yet.
    pub async fn save(&self) -> Result<(), String> {
        let file = self.file().ok_or_else(|| "Aucun fichier".to_string())?;
        let format = format_of(&file);
        let imp = self.imp();

        let mut doc = imp.editor.document();
        {
            let mut meta = imp.metadata.borrow_mut();
            if meta.title.is_none() {
                meta.title = file
                    .basename()
                    .and_then(|b| b.file_stem().map(|s| s.to_string_lossy().into_owned()))
                    .filter(|s| !s.is_empty());
            }
            doc.metadata = meta.clone();
        }

        let bytes = gio::spawn_blocking(move || formats::write_bytes(&doc, format))
            .await
            .map_err(|_| "Échec de l'enregistrement : erreur interne".to_string())?
            .map_err(|e| format!("Échec de l'enregistrement : {e}"))?;

        file.replace_contents_future(bytes, None, false, gio::FileCreateFlags::REPLACE_DESTINATION)
            .await
            .map_err(|(_, e)| format!("Échec de l'enregistrement : {e}"))?;

        imp.editor.buffer().set_modified(false);
        self.notify("modified");
        Ok(())
    }

    /// Saves to `file` (format from extension) and makes it the current file.
    pub async fn save_as(&self, file: gio::File) -> Result<(), String> {
        let previous = self.file();
        self.set_file(Some(file));
        let res = self.save().await;
        if res.is_err() {
            self.set_file(previous);
        }
        res
    }

    /// Glide-in animation: fade + slight upward slide of the page.
    fn play_entrance(&self) {
        let imp = self.imp();
        if let Some(old) = imp.enter_anim.take() {
            old.skip();
        }
        let weak = self.downgrade();
        let target = adw::CallbackAnimationTarget::new(move |v| {
            if let Some(page) = weak.upgrade() {
                let imp = page.imp();
                imp.clamp.set_opacity(v);
                let m = BOX_MARGIN_START_FROM as f64
                    + (BOX_MARGIN_TOP - BOX_MARGIN_START_FROM) as f64 * v;
                imp.content.set_margin_top(m.round() as i32);
            }
        });
        let anim = adw::TimedAnimation::new(&imp.clamp, 0.0, 1.0, 350, target);
        anim.set_easing(adw::Easing::EaseOutCubic);
        let weak = self.downgrade();
        anim.connect_done(move |_| {
            if let Some(page) = weak.upgrade() {
                page.imp().clamp.set_opacity(1.0);
                page.imp().content.set_margin_top(BOX_MARGIN_TOP);
            }
        });
        imp.clamp.set_opacity(0.0);
        imp.content.set_margin_top(BOX_MARGIN_START_FROM);
        anim.play();
        *imp.enter_anim.borrow_mut() = Some(anim);
    }

    fn schedule_scroll(&self) {
        let imp = self.imp();
        if imp.scroll_pending.replace(true) {
            return;
        }
        let weak = self.downgrade();
        glib::idle_add_local_once(move || {
            if let Some(page) = weak.upgrade() {
                page.imp().scroll_pending.set(false);
                page.scroll_to_cursor();
            }
        });
    }

    fn scroll_to_cursor(&self) {
        let imp = self.imp();
        if !imp.editor.is_mapped() || !imp.editor.has_focus() {
            return;
        }
        let buffer = imp.editor.buffer();
        let iter = buffer.iter_at_mark(&buffer.get_insert());
        let rect = imp.editor.iter_location(&iter);
        let (wx, wy) = imp
            .editor
            .buffer_to_window_coords(gtk::TextWindowType::Widget, rect.x(), rect.y());
        let Some(p) = imp
            .editor
            .compute_point(&imp.content, &gtk::graphene::Point::new(wx as f32, wy as f32))
        else {
            return;
        };
        let top = p.y() as f64;
        let bottom = top + rect.height() as f64;

        let adj = imp.scrolled.vadjustment();
        let value = adj.value();
        let page = adj.page_size();
        let max = (adj.upper() - page).max(adj.lower());
        const EDGE: f64 = 48.0;

        let target = if bottom > value + page - EDGE {
            bottom - page * 0.7
        } else if top < value + EDGE {
            top - page * 0.3
        } else {
            return;
        };
        let target = target.clamp(adj.lower(), max);
        if (target - value).abs() < 1.0 {
            return;
        }

        if let Some(old) = imp.scroll_anim.take() {
            old.skip();
        }
        let adj2 = adj.clone();
        let cb = adw::CallbackAnimationTarget::new(move |v| adj2.set_value(v));
        let anim = adw::TimedAnimation::new(&imp.scrolled, value, target, 180, cb);
        anim.set_easing(adw::Easing::EaseOutCubic);
        anim.play();
        *imp.scroll_anim.borrow_mut() = Some(anim);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    // Run from crate::gtk_tests (GTK is single-threaded).
    pub(crate) fn page_basics() {
        if gtk::init().is_err() || adw::init().is_err() {
            return;
        }
        let page = DocumentPage::new();
        assert_eq!(page.title(), "Nouveau document");
        assert!(!page.is_modified());
        assert!(page.is_blank());
        page.editor().buffer().set_text("x");
        page.editor().buffer().set_modified(true);
        assert!(page.is_modified());
        assert!(!page.is_blank());
    }
}
