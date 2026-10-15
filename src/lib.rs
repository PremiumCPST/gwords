pub mod application;
pub mod document;
pub mod document_page;
pub mod editor;
pub mod formats;
pub mod settings;
pub mod theme;
pub mod ui;
pub mod window;

/// GTK may only be initialized and used from a single thread, while libtest
/// runs each test on its own thread: all GTK-dependent checks run here.
#[cfg(test)]
mod gtk_tests {
    #[test]
    fn all_gtk_checks() {
        if gtk::init().is_err() || adw::init().is_err() {
            eprintln!("GTK indisponible : vérifications GTK ignorées");
            return;
        }
        crate::editor::convert::tests::roundtrip();
        crate::editor::tests::editor_behaviour();
        crate::document_page::tests::page_basics();
        crate::ui::format_bar::tests::gtk_checks();
        crate::ui::search_bar::tests::gtk_search_bar();
        crate::editor::table::tests::gtk_table();
        crate::ui::advanced_bar::tests::bar_builds();
        crate::ui::stats_dialog::tests::dialog_builds();
        crate::ui::special_chars::tests::gtk_checks();
    }
}
