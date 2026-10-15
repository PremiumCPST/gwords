use crate::document::{Document, ListKind};
use crate::formats::FormatError;

/// Windows-1252 mapping for bytes 0x80..=0x9F (the rest of the range is identical to Latin-1).
const CP1252_HIGH: [char; 32] = [
    '\u{20AC}', '\u{0081}', '\u{201A}', '\u{0192}', '\u{201E}', '\u{2026}', '\u{2020}', '\u{2021}',
    '\u{02C6}', '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{008D}', '\u{017D}', '\u{008F}',
    '\u{0090}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}', '\u{2013}', '\u{2014}',
    '\u{02DC}', '\u{2122}', '\u{0161}', '\u{203A}', '\u{0153}', '\u{009D}', '\u{017E}', '\u{0178}',
];

const UTF8_BOM: &[u8] = &[0xEF, 0xBB, 0xBF];

pub fn read(bytes: &[u8]) -> Result<Document, FormatError> {
    let bytes = bytes.strip_prefix(UTF8_BOM).unwrap_or(bytes);
    let text = match std::str::from_utf8(bytes) {
        Ok(s) => s.to_owned(),
        Err(_) => decode_windows_1252(bytes),
    };
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    Ok(Document::from_plain_text(&normalized))
}

pub fn write(doc: &Document) -> Result<Vec<u8>, FormatError> {
    // A trailing empty plain paragraph is what reading "...\n" produces; it is the
    // file's final newline, not a visible blank line, so it is not written twice.
    let mut paragraphs = doc.paragraphs.as_slice();
    if let [rest @ .., last] = paragraphs {
        if !rest.is_empty() && last.list.is_none() && last.plain_text().is_empty() {
            paragraphs = rest;
        }
    }

    let mut out = String::new();
    let mut number = 0u32;
    for paragraph in paragraphs {
        let text = paragraph.plain_text().replace('\u{2028}', "\n");
        match &paragraph.list {
            Some(ListKind::Bullet) => {
                number = 0;
                out.push_str("• ");
            }
            Some(ListKind::Numbered) => {
                number += 1;
                out.push_str(&format!("{number}. "));
            }
            None => number = 0,
        }
        out.push_str(&text);
        out.push('\n');
    }
    Ok(out.into_bytes())
}

/// Decodes bytes as Windows-1252. Every byte maps to a character, so this never fails.
fn decode_windows_1252(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|&b| match b {
            0x80..=0x9F => CP1252_HIGH[(b - 0x80) as usize],
            _ => char::from(b),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Paragraph, Run};

    fn para(text: &str, list: Option<ListKind>) -> Paragraph {
        Paragraph {
            list,
            runs: vec![Run::plain(text)],
            ..Default::default()
        }
    }

    fn doc(paragraphs: Vec<Paragraph>) -> Document {
        Document { paragraphs, ..Default::default() }
    }

    fn text_of(doc: &Document) -> String {
        doc.paragraphs.iter().map(|p| p.plain_text()).collect::<Vec<_>>().join("\n")
    }

    #[test]
    fn read_strips_bom_and_normalizes_newlines() {
        let mut bytes = UTF8_BOM.to_vec();
        bytes.extend_from_slice("a\r\nb\rc\n".as_bytes());
        let doc = read(&bytes).unwrap();
        assert_eq!(text_of(&doc), "a\nb\nc\n");
        assert_eq!(doc.paragraphs.len(), 4);
    }

    #[test]
    fn read_utf8_accents() {
        let doc = read("café été".as_bytes()).unwrap();
        assert_eq!(text_of(&doc), "café été");
    }

    #[test]
    fn read_invalid_utf8_falls_back_to_windows_1252() {
        // "café" in Latin-1, then "€" and a curly quote in Windows-1252.
        let doc = read(b"caf\xE9 \x80 \x93x\x94").unwrap();
        assert_eq!(text_of(&doc), "caf\u{e9} \u{20ac} \u{201c}x\u{201d}");
    }

    #[test]
    fn write_lists_and_trailing_newline() {
        let d = doc(vec![
            para("Intro", None),
            para("un", Some(ListKind::Bullet)),
            para("deux", Some(ListKind::Bullet)),
            para("premier", Some(ListKind::Numbered)),
            para("second", Some(ListKind::Numbered)),
            para("fin", None),
        ]);
        let out = String::from_utf8(write(&d).unwrap()).unwrap();
        assert_eq!(out, "Intro\n• un\n• deux\n1. premier\n2. second\nfin\n");
    }

    #[test]
    fn write_numbering_restarts_after_other_paragraph() {
        let d = doc(vec![
            para("a", Some(ListKind::Numbered)),
            para("b", None),
            para("c", Some(ListKind::Numbered)),
        ]);
        let out = String::from_utf8(write(&d).unwrap()).unwrap();
        assert_eq!(out, "1. a\nb\n1. c\n");
    }

    #[test]
    fn write_converts_line_separator_to_newline() {
        let d = doc(vec![para("ligne1\u{2028}ligne2", None)]);
        let out = String::from_utf8(write(&d).unwrap()).unwrap();
        assert_eq!(out, "ligne1\nligne2\n");
    }

    #[test]
    fn roundtrip_plain_text() {
        let original = "Bonjour\n\nTabulé\there\nfin\n";
        let d = read(original.as_bytes()).unwrap();
        let out = String::from_utf8(write(&d).unwrap()).unwrap();
        assert_eq!(out, original);
    }
}
