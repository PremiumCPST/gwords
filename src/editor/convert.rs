//! Conversion between the document model and a `gtk::TextBuffer` using the
//! tag conventions of `editor::tags`.
//!
//! Round-trip guarantee: for any *normalized* document `doc` (adjacent runs
//! with identical style merged, no empty runs, no `'\n'` inside runs, font
//! sizes rounded to 0.5pt), `buffer_to_document(load_into_buffer(doc)) == doc`
//! (metadata excepted: it is left at its default value).

use std::collections::HashMap;

use gtk::prelude::*;
use gtk::{TextBuffer, TextChildAnchor, TextTag};

use super::tags;
use super::table::TableWidget;
use crate::document::{Block, CharStyle, Document, ListKind, Paragraph, Rgb, Run, Table};

/// Names of the character-level tags for a style.
fn char_tag_names(style: &CharStyle) -> Vec<String> {
    let mut v = Vec::new();
    if style.bold {
        v.push(tags::BOLD.to_string());
    }
    if style.italic {
        v.push(tags::ITALIC.to_string());
    }
    if style.underline {
        v.push(tags::UNDERLINE.to_string());
    }
    if style.strikethrough {
        v.push(tags::STRIKETHROUGH.to_string());
    }
    if let Some(t) = tags::vertical_tag(style.vertical) {
        v.push(t.to_string());
    }
    if let Some(f) = &style.font_family {
        v.push(tags::font_tag_name(f));
    }
    if let Some(s) = style.font_size {
        v.push(tags::size_tag_name(s));
    }
    if let Some(c) = style.color {
        v.push(tags::color_tag_name(c));
    }
    if let Some(c) = style.highlight {
        v.push(tags::highlight_tag_name(c));
    }
    if let Some(u) = &style.link {
        v.push(tags::link_tag_name(u));
    }
    v
}

/// Clears `buffer` and fills it with the blocks of `doc`. For each table,
/// inserts a line containing exactly one TextChildAnchor (lines are separated by '\n'
/// like paragraphs) and returns the anchors with their tables, in document order.
/// The caller (Editor) creates a TableWidget per anchor (view.add_child_at_anchor).
pub fn load_into_buffer(buffer: &TextBuffer, doc: &Document) -> Vec<(TextChildAnchor, Table)> {
    let items: Vec<Item> = doc
        .blocks
        .iter()
        .map(|b| match b {
            Block::Paragraph(p) => Item::P(p),
            Block::Table(t) => Item::T(t),
        })
        .collect();
    load_items(buffer, &items)
}

/// Paragraph-only variant used for table cells: fills `buffer` with `paragraphs`.
pub fn load_paragraphs(buffer: &TextBuffer, paragraphs: &[Paragraph]) {
    let items: Vec<Item> = paragraphs.iter().map(Item::P).collect();
    load_items(buffer, &items);
}

enum Item<'a> {
    P(&'a Paragraph),
    T(&'a Table),
}

fn load_items(buffer: &TextBuffer, items: &[Item]) -> Vec<(TextChildAnchor, Table)> {
    tags::install_tags(buffer);

    let mut text = String::new();
    let mut len: i32 = 0; // length in chars
    // (start, end, tag name) in char offsets
    let mut ranges: Vec<(i32, i32, String)> = Vec::new();
    let mut anchors_at: Vec<(i32, &Table)> = Vec::new();
    let mut numbered = 0usize;

    let push = |text: &mut String, len: &mut i32, s: &str| {
        text.push_str(s);
        *len += s.chars().count() as i32;
    };

    let last = items.len().saturating_sub(1);
    for (i, item) in items.iter().enumerate() {
        let p = match item {
            Item::T(t) => {
                numbered = 0;
                anchors_at.push((len, t));
                push(&mut text, &mut len, "\u{FFFC}");
                if i != last {
                    push(&mut text, &mut len, "\n");
                }
                continue;
            }
            Item::P(p) => *p,
        };
        let line_start = len;
        match p.list {
            Some(kind) => {
                let marker = match kind {
                    ListKind::Bullet => {
                        numbered = 0;
                        tags::BULLET_MARKER.to_string()
                    }
                    ListKind::Numbered => {
                        numbered += 1;
                        tags::numbered_marker(numbered)
                    }
                };
                let s = len;
                push(&mut text, &mut len, &marker);
                ranges.push((s, len, tags::LIST_MARKER.to_string()));
            }
            None => numbered = 0,
        }
        for run in &p.runs {
            if run.text.is_empty() {
                continue;
            }
            let s = len;
            if run.text.contains('\n') {
                push(&mut text, &mut len, &run.text.replace('\n', "\u{2028}"));
            } else {
                push(&mut text, &mut len, &run.text);
            }
            for name in char_tag_names(&run.style) {
                ranges.push((s, len, name));
            }
        }
        if i != last {
            push(&mut text, &mut len, "\n");
        }
        if line_start != len {
            let mut names: Vec<&'static str> = Vec::new();
            names.extend(tags::paragraph_tag(p.style));
            names.extend(tags::alignment_tag(p.alignment));
            names.extend(p.list.map(tags::list_tag));
            for n in names {
                ranges.push((line_start, len, n.to_string()));
            }
        }
    }

    buffer.set_text(&text);

    let mut cache: HashMap<String, Option<TextTag>> = HashMap::new();
    for (s, e, name) in ranges {
        let tag = cache
            .entry(name.clone())
            .or_insert_with(|| tags::ensure_tag(buffer, &name))
            .clone();
        if let Some(tag) = tag {
            buffer.apply_tag(&tag, &buffer.iter_at_offset(s), &buffer.iter_at_offset(e));
        }
    }

    // Replace each placeholder by a real child anchor (same width: offsets stay valid).
    let mut out = Vec::new();
    for (off, table) in anchors_at {
        let mut start = buffer.iter_at_offset(off);
        let mut end = buffer.iter_at_offset(off + 1);
        buffer.delete(&mut start, &mut end);
        let mut it = buffer.iter_at_offset(off);
        let anchor = buffer.create_child_anchor(&mut it);
        out.push((anchor, table.clone()));
    }
    out
}

/// Reads the buffer back. A line made of exactly one child anchor whose first
/// widget is a `TableWidget` becomes `Block::Table`; an anchor with no
/// TableWidget is skipped.
pub fn buffer_to_document(buffer: &TextBuffer) -> Document {
    let mut blocks = Vec::new();
    for line in 0..buffer.line_count() {
        let Some(start) = buffer.iter_at_line(line) else { break };
        if let Some(anchor) = start.child_anchor() {
            let mut end = start.clone();
            if !end.ends_line() {
                end.forward_to_line_end();
            }
            let mut after = start.clone();
            after.forward_char();
            if after == end {
                if let Some(w) = anchor.widgets().into_iter().find_map(|w| w.downcast::<TableWidget>().ok()) {
                    blocks.push(Block::Table(w.to_table()));
                }
                continue;
            }
        }
        blocks.push(Block::Paragraph(read_line(buffer, line)));
    }
    Document { metadata: Default::default(), blocks }
}

/// Reads the buffer as paragraphs (table cells). An empty buffer gives an
/// empty vector.
pub fn buffer_to_paragraphs(buffer: &TextBuffer) -> Vec<Paragraph> {
    if buffer.char_count() == 0 {
        return Vec::new();
    }
    (0..buffer.line_count()).map(|l| read_line(buffer, l)).collect()
}

fn read_line(buffer: &TextBuffer, line: i32) -> Paragraph {
    let mut para = Paragraph::default();
    let Some(start) = buffer.iter_at_line(line) else { return para };
    let mut end = start.clone();
    if !end.ends_line() {
        end.forward_to_line_end();
    }

    // Paragraph-level tags from the first char (or the '\n' of an empty line).
    if !start.is_end() {
        for tag in start.tags() {
            let Some(name) = tag.name() else { continue };
            let name = name.as_str();
            if let Some(s) = tags::paragraph_style_from_tag(name) {
                para.style = s;
            } else if let Some(a) = tags::alignment_from_tag(name) {
                para.alignment = a;
            } else if let Some(l) = tags::list_from_tag(name) {
                para.list = Some(l);
            }
        }
    }

    let mut pos = start.clone();
    while pos < end {
        let mut next = pos.clone();
        if !next.forward_to_tag_toggle(None::<&TextTag>) || next > end {
            next = end.clone();
        }
        let mut style = CharStyle::default();
        let mut marker = false;
        for tag in pos.tags() {
            let Some(name) = tag.name() else { continue };
            let name = name.as_str();
            if name == tags::LIST_MARKER {
                marker = true;
            } else if !tags::is_transient_tag(name) {
                apply_tag_name(&mut style, name);
            }
        }
        if !marker {
            let seg: String = buffer
                .text(&pos, &next, false)
                .chars()
                .filter(|c| *c != '\u{FFFC}' && *c != '\n')
                .collect();
            if !seg.is_empty() {
                match para.runs.last_mut() {
                    Some(last) if last.style == style => last.text.push_str(&seg),
                    _ => para.runs.push(Run { text: seg, style }),
                }
            }
        }
        pos = next;
    }
    para
}

fn apply_tag_name(style: &mut CharStyle, name: &str) {
    match name {
        tags::BOLD => style.bold = true,
        tags::ITALIC => style.italic = true,
        tags::UNDERLINE => style.underline = true,
        tags::STRIKETHROUGH => style.strikethrough = true,
        tags::SUPERSCRIPT => style.vertical = crate::document::VerticalAlign::Superscript,
        tags::SUBSCRIPT => style.vertical = crate::document::VerticalAlign::Subscript,
        _ => {
            if let Some(f) = name.strip_prefix(tags::FONT_PREFIX) {
                style.font_family = Some(f.to_string());
            } else if let Some(s) = name.strip_prefix(tags::SIZE_PREFIX) {
                style.font_size = s.parse().ok();
            } else if let Some(c) = name.strip_prefix(tags::COLOR_PREFIX) {
                style.color = Rgb::from_hex(c);
            } else if let Some(c) = name.strip_prefix(tags::HIGHLIGHT_PREFIX) {
                style.highlight = Rgb::from_hex(c);
            } else if let Some(u) = name.strip_prefix(tags::LINK_PREFIX) {
                style.link = Some(u.to_string());
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::document::{Alignment, ParagraphStyle, VerticalAlign};

    fn run(t: &str, f: impl FnOnce(&mut CharStyle)) -> Run {
        let mut style = CharStyle::default();
        f(&mut style);
        Run { text: t.into(), style }
    }

    fn para(style: ParagraphStyle, alignment: Alignment, list: Option<ListKind>, runs: Vec<Run>) -> Paragraph {
        Paragraph { style, alignment, list, runs }
    }

    fn doc(ps: Vec<Paragraph>) -> Document {
        Document { blocks: ps.into_iter().map(Block::Paragraph).collect(), ..Default::default() }
    }

    // Run from crate::gtk_tests (GTK is single-threaded).
    pub(crate) fn roundtrip() {
        if gtk::init().is_err() {
            return;
        }
        use ParagraphStyle::*;
        let n = Alignment::Left;
        let docs = vec![
            doc(vec![para(Normal, n, None, vec![])]),
            doc(vec![
                    para(Title, Alignment::Center, None, vec![Run::plain("Titre")]),
                    para(Heading1, n, None, vec![run("Gras", |s| s.bold = true), Run::plain(" et "), run("it", |s| {
                        s.italic = true;
                        s.underline = true;
                        s.strikethrough = true;
                    })]),
                    para(Normal, Alignment::Justify, None, vec![
                        run("rouge", |s| {
                            s.color = Some(Rgb(255, 0, 0));
                            s.highlight = Some(Rgb(1, 2, 3));
                        }),
                        run("serif", |s| {
                            s.font_family = Some("Noto Serif".into());
                            s.font_size = Some(13.5);
                        }),
                        run("x2", |s| s.vertical = VerticalAlign::Superscript),
                        run("x", |s| s.vertical = VerticalAlign::Subscript),
                        run("lien", |s| s.link = Some("https://gnome.org".into())),
                    ]),
                    para(Quote, Alignment::Right, None, vec![Run::plain("a\tb\u{2028}c")]),
                    para(Code, n, None, vec![]),
                    para(Normal, n, None, vec![Run::plain("héllo 🎉 日本語")]),
                    para(Normal, n, None, vec![]),
                ]),
            doc(vec![
                    para(Normal, n, Some(ListKind::Bullet), vec![Run::plain("puce")]),
                    para(Normal, n, Some(ListKind::Numbered), vec![Run::plain("un")]),
                    para(Normal, n, Some(ListKind::Numbered), vec![]),
                    para(Normal, n, Some(ListKind::Numbered), vec![run("trois", |s| s.bold = true)]),
                    para(Normal, n, None, vec![Run::plain("rupture")]),
                    para(Heading2, Alignment::Center, Some(ListKind::Numbered), vec![Run::plain("repart")]),
                    para(Normal, n, Some(ListKind::Bullet), vec![]),
                ]),
            doc(vec![]),
        ];
        for (i, doc) in docs.iter().enumerate() {
            let buf = TextBuffer::new(None);
            load_into_buffer(&buf, doc);
            let back = buffer_to_document(&buf);
            if doc.blocks.is_empty() {
                assert_eq!(back.blocks.len(), 1);
                continue;
            }
            assert_eq!(back, doc.normalized(), "doc {i}");
            // Reloading into the same buffer works too.
            load_into_buffer(&buf, doc);
            assert_eq!(buffer_to_document(&buf), doc.normalized(), "reload doc {i}");
        }

        let buf = TextBuffer::new(None);
        load_into_buffer(&buf, &docs[2]);
        let text = buf.text(&buf.start_iter(), &buf.end_iter(), false);
        assert_eq!(text, "• puce\n1. un\n2. \n3. trois\nrupture\n1. repart\n• ");

        // Cells.
        let cell = vec![para(Normal, n, None, vec![Run::plain("a")]), para(Normal, n, Some(ListKind::Bullet), vec![])];
        let buf = TextBuffer::new(None);
        load_paragraphs(&buf, &cell);
        assert_eq!(buffer_to_paragraphs(&buf), cell);
        load_paragraphs(&buf, &[]);
        assert!(buffer_to_paragraphs(&buf).is_empty());

        // Tables: anchors at the expected lines; widget-less anchors are skipped.
        let mut t1 = Table::new(2, 2);
        t1.rows[0].cells[0].paragraphs = cell.clone();
        let t2 = Table::new(1, 1);
        let p = |s: &str| Block::Paragraph(para(Normal, n, None, vec![Run::plain(s)]));
        let d = Document {
            blocks: vec![Block::Table(t1.clone()), p("x"), Block::Table(t2.clone()), p("y"), Block::Table(t1.clone())],
            ..Default::default()
        };
        let buf = TextBuffer::new(None);
        let anchors = load_into_buffer(&buf, &d);
        assert_eq!(anchors.len(), 3);
        assert_eq!(anchors[0].1, t1);
        assert_eq!(anchors[1].1, t2);
        assert_eq!(buf.line_count(), 5);
        for (anchor, line) in [(&anchors[0].0, 0), (&anchors[1].0, 2), (&anchors[2].0, 4)] {
            let it = buf.iter_at_line(line).unwrap();
            assert_eq!(it.child_anchor().as_ref(), Some(anchor));
        }
        assert_eq!(buf.iter_at_line(1).unwrap().child_anchor(), None);
        assert_eq!(buffer_to_document(&buf).blocks, vec![p("x"), p("y")]);
        // Reload clears the old anchors.
        let anchors = load_into_buffer(&buf, &doc(vec![para(Normal, n, None, vec![Run::plain("z")])]));
        assert!(anchors.is_empty());
        assert_eq!(buffer_to_document(&buf).blocks, vec![p("z")]);
    }
}
