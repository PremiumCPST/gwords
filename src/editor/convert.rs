//! Conversion between the document model and a `gtk::TextBuffer` using the
//! tag conventions of `editor::tags`.
//!
//! Round-trip guarantee: for any *normalized* document `doc` (adjacent runs
//! with identical style merged, no empty runs, no `'\n'` inside runs, font
//! sizes rounded to 0.5pt), `buffer_to_document(load_into_buffer(doc)) == doc`
//! (metadata excepted: it is left at its default value).

use std::collections::HashMap;

use gtk::prelude::*;
use gtk::{TextBuffer, TextTag};

use super::tags;
use crate::document::{CharStyle, Document, ListKind, Paragraph, Rgb, Run};

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
    v
}

/// Clears `buffer` and fills it with `doc`. Calls `tags::install_tags` first.
pub fn load_into_buffer(buffer: &TextBuffer, doc: &Document) {
    tags::install_tags(buffer);

    let mut text = String::new();
    let mut len: i32 = 0; // length in chars
    // (start, end, tag name) in char offsets
    let mut ranges: Vec<(i32, i32, String)> = Vec::new();
    let mut numbered = 0usize;

    let push = |text: &mut String, len: &mut i32, s: &str| {
        text.push_str(s);
        *len += s.chars().count() as i32;
    };

    let last = doc.paragraphs.len().saturating_sub(1);
    for (i, p) in doc.paragraphs.iter().enumerate() {
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
}

/// Reads the whole buffer back into a document model.
pub fn buffer_to_document(buffer: &TextBuffer) -> Document {
    let mut paragraphs = Vec::new();
    let line_count = buffer.line_count();
    for line in 0..line_count {
        let Some(start) = buffer.iter_at_line(line) else { break };
        let mut end = start.clone();
        if !end.ends_line() {
            end.forward_to_line_end();
        }
        let mut para = Paragraph::default();

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
        paragraphs.push(para);
    }
    Document { metadata: Default::default(), paragraphs }
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

    // Run from crate::gtk_tests (GTK is single-threaded).
    pub(crate) fn roundtrip() {
        if gtk::init().is_err() {
            return;
        }
        use ParagraphStyle::*;
        let n = Alignment::Left;
        let docs = vec![
            Document { paragraphs: vec![para(Normal, n, None, vec![])], ..Default::default() },
            Document {
                paragraphs: vec![
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
                    ]),
                    para(Quote, Alignment::Right, None, vec![Run::plain("a\tb\u{2028}c")]),
                    para(Code, n, None, vec![]),
                    para(Normal, n, None, vec![Run::plain("héllo 🎉 日本語")]),
                    para(Normal, n, None, vec![]),
                ],
                ..Default::default()
            },
            Document {
                paragraphs: vec![
                    para(Normal, n, Some(ListKind::Bullet), vec![Run::plain("puce")]),
                    para(Normal, n, Some(ListKind::Numbered), vec![Run::plain("un")]),
                    para(Normal, n, Some(ListKind::Numbered), vec![]),
                    para(Normal, n, Some(ListKind::Numbered), vec![run("trois", |s| s.bold = true)]),
                    para(Normal, n, None, vec![Run::plain("rupture")]),
                    para(Heading2, Alignment::Center, Some(ListKind::Numbered), vec![Run::plain("repart")]),
                    para(Normal, n, Some(ListKind::Bullet), vec![]),
                ],
                ..Default::default()
            },
            Document { paragraphs: vec![], ..Default::default() },
        ];
        for (i, doc) in docs.iter().enumerate() {
            let buf = TextBuffer::new(None);
            load_into_buffer(&buf, doc);
            let back = buffer_to_document(&buf);
            if doc.paragraphs.is_empty() {
                assert_eq!(back.paragraphs.len(), 1);
                continue;
            }
            assert_eq!(&back, doc, "doc {i}");
            // Reloading into the same buffer works too.
            load_into_buffer(&buf, doc);
            assert_eq!(&buffer_to_document(&buf), doc, "reload doc {i}");
        }

        let buf = TextBuffer::new(None);
        load_into_buffer(&buf, &docs[2]);
        let text = buf.text(&buf.start_iter(), &buf.end_iter(), false);
        assert_eq!(text, "• puce\n1. un\n2. \n3. trois\nrupture\n1. repart\n• ");
    }
}
