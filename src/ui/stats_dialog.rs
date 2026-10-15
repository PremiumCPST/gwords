//! "Statistiques du document" dialog.

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use crate::editor::Statistics;

/// Groups digits by three with a narrow no-break space (U+202F).
pub(crate) fn group_thousands(n: usize) -> String {
    let digits = n.to_string();
    let len = digits.len();
    let mut out = String::with_capacity(len + len / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (len - i) % 3 == 0 {
            out.push('\u{202F}');
        }
        out.push(c);
    }
    out
}

/// Human readable reading time.
pub(crate) fn reading_time_text(minutes: usize) -> String {
    if minutes <= 1 {
        "moins d'une minute".to_string()
    } else if minutes < 60 {
        format!("≈ {minutes} min")
    } else {
        format!("≈ {} h {:02}", minutes / 60, minutes % 60)
    }
}

fn row(title: &str) -> (adw::ActionRow, gtk::Label) {
    let row = adw::ActionRow::builder().title(title).build();
    let label = gtk::Label::new(Some("0"));
    label.add_css_class("numeric");
    label.add_css_class("stats-number");
    label.add_css_class("dim-label");
    label.set_valign(gtk::Align::Center);
    row.add_suffix(&label);
    (row, label)
}

/// Builds the dialog; the caller presents it with `.present(Some(&parent))`.
pub fn stats_dialog(stats: &Statistics) -> adw::Dialog {
    let group = adw::PreferencesGroup::new();
    let values: [(&str, usize); 5] = [
        ("Mots", stats.words),
        ("Caractères (espaces compris)", stats.characters),
        ("Caractères (sans les espaces)", stats.characters_no_spaces),
        ("Paragraphes", stats.paragraphs),
        ("Tableaux", stats.tables),
    ];
    let mut labels = Vec::new();
    for (title, value) in values {
        let (r, l) = row(title);
        group.add(&r);
        labels.push((l, value));
    }
    let (r, reading) = row("Temps de lecture estimé");
    reading.set_label(&reading_time_text(stats.reading_minutes));
    group.add(&r);

    let page = adw::PreferencesPage::new();
    page.add(&group);

    let header = adw::HeaderBar::new();
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&page));

    let dialog = adw::Dialog::builder()
        .title("Statistiques")
        .content_width(360)
        .child(&toolbar)
        .build();

    // Count-up animation, once, when the dialog is first shown.
    let started = Rc::new(Cell::new(false));
    let labels = Rc::new(labels);
    dialog.connect_map(move |dialog| {
        if started.replace(true) {
            return;
        }
        let labels = labels.clone();
        for (l, _) in labels.iter() {
            l.set_label("0");
        }
        let target = adw::CallbackAnimationTarget::new(glib::clone!(
            #[strong]
            labels,
            move |t| {
                for (l, v) in labels.iter() {
                    l.set_label(&group_thousands((*v as f64 * t).round() as usize));
                }
            }
        ));
        let anim = adw::TimedAnimation::builder()
            .widget(dialog)
            .value_from(0.0)
            .value_to(1.0)
            .duration(400)
            .easing(adw::Easing::EaseOutCubic)
            .target(&target)
            .build();
        anim.play();
    });
    dialog
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn thousands() {
        assert_eq!(group_thousands(0), "0");
        assert_eq!(group_thousands(999), "999");
        assert_eq!(group_thousands(1000), "1\u{202F}000");
        assert_eq!(group_thousands(1234567), "1\u{202F}234\u{202F}567");
    }

    #[test]
    fn reading() {
        assert_eq!(reading_time_text(0), "moins d'une minute");
        assert_eq!(reading_time_text(3), "≈ 3 min");
        assert_eq!(reading_time_text(65), "≈ 1 h 05");
    }
}

/// GTK-dependent check, to be called from `crate::gtk_tests`.
#[cfg(test)]
pub(crate) mod tests {
    pub(crate) fn dialog_builds() {
        let d = super::stats_dialog(&crate::editor::Statistics::default());
        assert_eq!(gtk::prelude::ObjectExt::property::<String>(&d, "title"), "Statistiques");
    }
}
