//! TextBuffer tag conventions. THIS FILE IS A CONTRACT shared by:
//! `editor/mod.rs`, `editor/convert.rs`, `editor/typing_anim.rs`,
//! `editor/caret_anim.rs` and `ui/search_bar.rs`.
//!
//! Static tags are created by [`install_tags`]. Dynamic tags (font family, size,
//! colors) are named with a `prefix:value` scheme and created lazily by
//! [`ensure_tag`].
//!
//! Paragraph-level tags (paragraph style, alignment, list) must always cover a
//! whole line including its trailing '\n' (when present).
//!
//! Lists: a list paragraph starts with a marker text ("• " for bullets,
//! "1. ", "2. "... for numbered) tagged with [`LIST_MARKER`]; the marker text is
//! NOT part of the document model (converters strip/insert it).
//!
//! Transient tags (`anim-*`, `search-*`) are visual only and ignored by converters.

use gtk::prelude::*;
use gtk::{pango, TextBuffer, TextTag};

use crate::document::{Alignment, ListKind, ParagraphStyle, Rgb, VerticalAlign};

pub const BOLD: &str = "bold";
pub const ITALIC: &str = "italic";
pub const UNDERLINE: &str = "underline";
pub const STRIKETHROUGH: &str = "strikethrough";
pub const SUPERSCRIPT: &str = "superscript";
pub const SUBSCRIPT: &str = "subscript";

pub const PARA_TITLE: &str = "para-title";
pub const PARA_H1: &str = "para-h1";
pub const PARA_H2: &str = "para-h2";
pub const PARA_H3: &str = "para-h3";
pub const PARA_QUOTE: &str = "para-quote";
pub const PARA_CODE: &str = "para-code";

pub const ALIGN_CENTER: &str = "align-center";
pub const ALIGN_RIGHT: &str = "align-right";
pub const ALIGN_JUSTIFY: &str = "align-justify";

pub const LIST_BULLET: &str = "list-bullet";
pub const LIST_NUMBERED: &str = "list-numbered";
pub const LIST_MARKER: &str = "list-marker";

pub const SEARCH_MATCH: &str = "search-match";
pub const SEARCH_CURRENT: &str = "search-current";

pub const FONT_PREFIX: &str = "font:";
pub const SIZE_PREFIX: &str = "size:";
pub const COLOR_PREFIX: &str = "color:";
pub const HIGHLIGHT_PREFIX: &str = "highlight:";

pub const BULLET_MARKER: &str = "• ";

pub fn numbered_marker(n: usize) -> String {
    format!("{n}. ")
}

/// Tag for a paragraph style; `None` for `Normal`.
pub fn paragraph_tag(style: ParagraphStyle) -> Option<&'static str> {
    match style {
        ParagraphStyle::Normal => None,
        ParagraphStyle::Title => Some(PARA_TITLE),
        ParagraphStyle::Heading1 => Some(PARA_H1),
        ParagraphStyle::Heading2 => Some(PARA_H2),
        ParagraphStyle::Heading3 => Some(PARA_H3),
        ParagraphStyle::Quote => Some(PARA_QUOTE),
        ParagraphStyle::Code => Some(PARA_CODE),
    }
}

pub fn paragraph_style_from_tag(name: &str) -> Option<ParagraphStyle> {
    ParagraphStyle::ALL.into_iter().find(|s| paragraph_tag(*s) == Some(name))
}

/// Tag for an alignment; `None` for `Left`.
pub fn alignment_tag(a: Alignment) -> Option<&'static str> {
    match a {
        Alignment::Left => None,
        Alignment::Center => Some(ALIGN_CENTER),
        Alignment::Right => Some(ALIGN_RIGHT),
        Alignment::Justify => Some(ALIGN_JUSTIFY),
    }
}

pub fn alignment_from_tag(name: &str) -> Option<Alignment> {
    match name {
        ALIGN_CENTER => Some(Alignment::Center),
        ALIGN_RIGHT => Some(Alignment::Right),
        ALIGN_JUSTIFY => Some(Alignment::Justify),
        _ => None,
    }
}

pub fn list_tag(kind: ListKind) -> &'static str {
    match kind {
        ListKind::Bullet => LIST_BULLET,
        ListKind::Numbered => LIST_NUMBERED,
    }
}

pub fn list_from_tag(name: &str) -> Option<ListKind> {
    match name {
        LIST_BULLET => Some(ListKind::Bullet),
        LIST_NUMBERED => Some(ListKind::Numbered),
        _ => None,
    }
}

pub fn vertical_tag(v: VerticalAlign) -> Option<&'static str> {
    match v {
        VerticalAlign::Baseline => None,
        VerticalAlign::Superscript => Some(SUPERSCRIPT),
        VerticalAlign::Subscript => Some(SUBSCRIPT),
    }
}

pub fn font_tag_name(family: &str) -> String {
    format!("{FONT_PREFIX}{family}")
}

/// Size is rounded to 0.5pt to keep the number of tags bounded.
pub fn size_tag_name(points: f64) -> String {
    format!("{SIZE_PREFIX}{:.1}", (points * 2.0).round() / 2.0)
}

pub fn color_tag_name(c: Rgb) -> String {
    format!("{COLOR_PREFIX}{}", c.to_hex())
}

pub fn highlight_tag_name(c: Rgb) -> String {
    format!("{HIGHLIGHT_PREFIX}{}", c.to_hex())
}

/// True for tags that are purely visual and must be ignored by converters.
pub fn is_transient_tag(name: &str) -> bool {
    name.starts_with("anim-") || name.starts_with("search-")
}

fn rgba(c: Rgb) -> gtk::gdk::RGBA {
    gtk::gdk::RGBA::new(c.0 as f32 / 255.0, c.1 as f32 / 255.0, c.2 as f32 / 255.0, 1.0)
}

/// Tag margins REPLACE the view's margins (they don't add up), so margins of
/// indented paragraphs include the page padding.
const PAD: i32 = crate::theme::PAGE_PADDING_X;

/// Creates all static tags in the buffer's tag table. Idempotent.
pub fn install_tags(buffer: &TextBuffer) {
    let table = buffer.tag_table();
    let add = |tag: TextTag| {
        if table.lookup(tag.name().as_deref().unwrap_or_default()).is_none() {
            table.add(&tag);
        }
    };

    // Paragraph styles first: lowest priority.
    add(TextTag::builder().name(PARA_TITLE).scale(2.2).weight(800).pixels_above_lines(12).pixels_below_lines(18).build());
    add(TextTag::builder().name(PARA_H1).scale(1.75).weight(700).pixels_above_lines(18).pixels_below_lines(8).build());
    add(TextTag::builder().name(PARA_H2).scale(1.4).weight(700).pixels_above_lines(14).pixels_below_lines(6).build());
    add(TextTag::builder().name(PARA_H3).scale(1.2).weight(700).pixels_above_lines(10).pixels_below_lines(4).build());
    add(TextTag::builder()
        .name(PARA_QUOTE)
        .style(pango::Style::Italic)
        .left_margin(PAD + 32)
        .right_margin(PAD + 32)
        .pixels_above_lines(6)
        .pixels_below_lines(6)
        .build());
    add(TextTag::builder()
        .name(PARA_CODE)
        .family("monospace")
        .paragraph_background_rgba(&gtk::gdk::RGBA::new(0.5, 0.5, 0.5, 0.12))
        .left_margin(PAD)
        .right_margin(PAD)
        .build());

    add(TextTag::builder().name(ALIGN_CENTER).justification(gtk::Justification::Center).build());
    add(TextTag::builder().name(ALIGN_RIGHT).justification(gtk::Justification::Right).build());
    add(TextTag::builder().name(ALIGN_JUSTIFY).justification(gtk::Justification::Fill).build());

    add(TextTag::builder().name(LIST_BULLET).left_margin(PAD + 28).indent(-18).build());
    add(TextTag::builder().name(LIST_NUMBERED).left_margin(PAD + 28).indent(-22).build());
    add(TextTag::builder().name(LIST_MARKER).build());

    add(TextTag::builder().name(BOLD).weight(700).build());
    add(TextTag::builder().name(ITALIC).style(pango::Style::Italic).build());
    add(TextTag::builder().name(UNDERLINE).underline(pango::Underline::Single).build());
    add(TextTag::builder().name(STRIKETHROUGH).strikethrough(true).build());
    add(TextTag::builder().name(SUPERSCRIPT).rise(6 * pango::SCALE).scale(0.7).build());
    add(TextTag::builder().name(SUBSCRIPT).rise(-3 * pango::SCALE).scale(0.7).build());

    add(TextTag::builder()
        .name(SEARCH_MATCH)
        .background_rgba(&gtk::gdk::RGBA::new(0.96, 0.83, 0.18, 0.35))
        .build());
    add(TextTag::builder()
        .name(SEARCH_CURRENT)
        .background_rgba(&gtk::gdk::RGBA::new(0.96, 0.60, 0.10, 0.70))
        .build());
}

/// Returns the tag with this name, creating dynamic (`font:`, `size:`,
/// `color:`, `highlight:`) tags on demand. Returns `None` for an unknown
/// static name or an unparsable dynamic value.
pub fn ensure_tag(buffer: &TextBuffer, name: &str) -> Option<TextTag> {
    let table = buffer.tag_table();
    if let Some(tag) = table.lookup(name) {
        return Some(tag);
    }
    let tag = if let Some(family) = name.strip_prefix(FONT_PREFIX) {
        TextTag::builder().name(name).family(family).build()
    } else if let Some(size) = name.strip_prefix(SIZE_PREFIX) {
        let pt: f64 = size.parse().ok()?;
        TextTag::builder().name(name).size_points(pt).build()
    } else if let Some(hex) = name.strip_prefix(COLOR_PREFIX) {
        TextTag::builder().name(name).foreground_rgba(&rgba(Rgb::from_hex(hex)?)).build()
    } else if let Some(hex) = name.strip_prefix(HIGHLIGHT_PREFIX) {
        TextTag::builder().name(name).background_rgba(&rgba(Rgb::from_hex(hex)?)).build()
    } else {
        return None;
    };
    table.add(&tag);
    Some(tag)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(size_tag_name(11.26), "size:11.5");
        assert_eq!(color_tag_name(Rgb(255, 0, 16)), "color:ff0010");
        assert!(is_transient_tag("anim-fade-3"));
        assert!(!is_transient_tag("bold"));
        assert_eq!(paragraph_style_from_tag(PARA_H2), Some(ParagraphStyle::Heading2));
    }
}
