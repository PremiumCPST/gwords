//! Preferences dialog bound to `crate::settings`.

use adw::prelude::*;
use gtk::pango;

use crate::settings::{self, ColorScheme, Settings};

/// Applies a change to the stored settings and saves them.
fn update(change: impl FnOnce(&mut Settings)) {
    let mut s = settings::get();
    change(&mut s);
    settings::set(s);
}

fn scheme_index(scheme: ColorScheme) -> u32 {
    match scheme {
        ColorScheme::System => 0,
        ColorScheme::Light => 1,
        ColorScheme::Dark => 2,
    }
}

fn scheme_from_index(index: u32) -> ColorScheme {
    match index {
        1 => ColorScheme::Light,
        2 => ColorScheme::Dark,
        _ => ColorScheme::System,
    }
}

/// Builds the preferences dialog; caller presents it with `.present(Some(&window))`.
pub fn preferences_dialog() -> adw::PreferencesDialog {
    let current = settings::get();

    let dialog = adw::PreferencesDialog::new();
    dialog.set_title("Préférences");

    let page = adw::PreferencesPage::builder()
        .title("Général")
        .icon_name("preferences-system-symbolic")
        .build();

    // Apparence
    let appearance = adw::PreferencesGroup::builder()
        .title("Apparence")
        .build();

    let style = adw::ComboRow::builder().title("Style").build();
    let style_model = gtk::StringList::new(&["Système", "Clair", "Sombre"]);
    style.set_model(Some(&style_model));
    style.set_selected(scheme_index(current.color_scheme));
    style.connect_selected_notify(|row| {
        let scheme = scheme_from_index(row.selected());
        update(|s| s.color_scheme = scheme);
    });
    appearance.add(&style);

    let font_row = adw::ActionRow::builder().title("Police").build();
    let font_button = gtk::FontDialogButton::new(Some(gtk::FontDialog::new()));
    font_button.set_level(gtk::FontLevel::Family);
    font_button.set_valign(gtk::Align::Center);
    // A FontDescription built from a string would parse a trailing number as size,
    // so set the family explicitly.
    let mut font_desc = pango::FontDescription::new();
    font_desc.set_family(&current.font_family);
    font_button.set_font_desc(&font_desc);
    font_button.connect_font_desc_notify(|button| {
        if let Some(family) = button.font_desc().and_then(|d| d.family()) {
            let family = family.to_string();
            update(|s| s.font_family = family);
        }
    });
    font_row.add_suffix(&font_button);
    appearance.add(&font_row);

    let size_adjustment = gtk::Adjustment::new(current.font_size, 6.0, 72.0, 1.0, 10.0, 0.0);
    let size = adw::SpinRow::new(Some(&size_adjustment), 1.0, 0);
    size.set_title("Taille de police");
    size.connect_value_notify(|row| {
        let value = row.value();
        update(|s| s.font_size = value);
    });
    appearance.add(&size);
    page.add(&appearance);

    // Animations
    let animations = adw::PreferencesGroup::builder()
        .title("Animations")
        .description("Rendez l'écriture plus vivante")
        .build();

    let typing = adw::SwitchRow::builder()
        .title("Animation de frappe")
        .subtitle("Les caractères apparaissent comme de l'encre")
        .active(current.typing_animation)
        .build();
    typing.connect_active_notify(|row| {
        let on = row.is_active();
        update(|s| s.typing_animation = on);
    });
    animations.add(&typing);

    let caret = adw::SwitchRow::builder()
        .title("Curseur fluide")
        .subtitle("Le curseur glisse et pulse doucement")
        .active(current.caret_animation)
        .build();
    caret.connect_active_notify(|row| {
        let on = row.is_active();
        update(|s| s.caret_animation = on);
    });
    animations.add(&caret);
    page.add(&animations);

    // Interface
    let interface = adw::PreferencesGroup::builder()
        .title("Interface")
        .build();

    let status = adw::SwitchRow::builder()
        .title("Barre d'état")
        .active(current.show_status_bar)
        .build();
    status.connect_active_notify(|row| {
        let on = row.is_active();
        update(|s| s.show_status_bar = on);
    });
    interface.add(&status);
    page.add(&interface);

    dialog.add(&page);
    dialog
}
