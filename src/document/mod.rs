//! Format-agnostic rich text document model.
//!
//! This is the contract shared by the file format readers/writers
//! (`crate::formats`) and the editor (`crate::editor::convert`).
//!
//! Conventions:
//! - A document is a flat list of paragraphs.
//! - Inside a run, `'\t'` is a tab and `'\u{2028}'` (LINE SEPARATOR) is a soft
//!   line break (ODT `<text:line-break/>`, DOCX `<w:br/>`). Runs never contain `'\n'`.
//! - An empty paragraph has an empty `runs` vector (or runs with empty text).

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Document {
    pub metadata: Metadata,
    pub paragraphs: Vec<Paragraph>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Metadata {
    pub title: Option<String>,
    pub author: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Paragraph {
    pub style: ParagraphStyle,
    pub alignment: Alignment,
    pub list: Option<ListKind>,
    pub runs: Vec<Run>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum ParagraphStyle {
    #[default]
    Normal,
    Title,
    Heading1,
    Heading2,
    Heading3,
    Quote,
    Code,
}

impl ParagraphStyle {
    pub const ALL: [ParagraphStyle; 7] = [
        ParagraphStyle::Normal,
        ParagraphStyle::Title,
        ParagraphStyle::Heading1,
        ParagraphStyle::Heading2,
        ParagraphStyle::Heading3,
        ParagraphStyle::Quote,
        ParagraphStyle::Code,
    ];

    /// Human readable (French) label for UI.
    pub fn label(self) -> &'static str {
        match self {
            ParagraphStyle::Normal => "Texte normal",
            ParagraphStyle::Title => "Titre",
            ParagraphStyle::Heading1 => "Titre 1",
            ParagraphStyle::Heading2 => "Titre 2",
            ParagraphStyle::Heading3 => "Titre 3",
            ParagraphStyle::Quote => "Citation",
            ParagraphStyle::Code => "Code",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Alignment {
    #[default]
    Left,
    Center,
    Right,
    Justify,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ListKind {
    Bullet,
    Numbered,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Run {
    pub text: String,
    pub style: CharStyle,
}

impl Run {
    pub fn plain(text: impl Into<String>) -> Self {
        Run { text: text.into(), style: CharStyle::default() }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct CharStyle {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
    pub vertical: VerticalAlign,
    /// Font family name, e.g. "Cantarell". `None` = document default.
    pub font_family: Option<String>,
    /// Font size in points. `None` = default for the paragraph style.
    pub font_size: Option<f64>,
    pub color: Option<Rgb>,
    pub highlight: Option<Rgb>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum VerticalAlign {
    #[default]
    Baseline,
    Superscript,
    Subscript,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    /// "rrggbb" lowercase, no '#'.
    pub fn to_hex(self) -> String {
        format!("{:02x}{:02x}{:02x}", self.0, self.1, self.2)
    }

    /// Parses "rrggbb" or "#rrggbb" (case-insensitive).
    pub fn from_hex(s: &str) -> Option<Rgb> {
        let s = s.trim().trim_start_matches('#');
        if s.len() != 6 || !s.is_ascii() {
            return None;
        }
        let p = |i: usize| u8::from_str_radix(&s[i..i + 2], 16).ok();
        Some(Rgb(p(0)?, p(2)?, p(4)?))
    }
}

impl Paragraph {
    pub fn plain_text(&self) -> String {
        self.runs.iter().map(|r| r.text.as_str()).collect()
    }
}

impl Document {
    /// Builds a document of plain paragraphs, one per line.
    pub fn from_plain_text(text: &str) -> Document {
        let paragraphs = text
            .split('\n')
            .map(|line| Paragraph {
                runs: if line.is_empty() { vec![] } else { vec![Run::plain(line.trim_end_matches('\r'))] },
                ..Default::default()
            })
            .collect();
        Document { metadata: Metadata::default(), paragraphs }
    }

    pub fn plain_text(&self) -> String {
        self.paragraphs.iter().map(Paragraph::plain_text).collect::<Vec<_>>().join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb_hex_roundtrip() {
        let c = Rgb(0x12, 0xab, 0xff);
        assert_eq!(c.to_hex(), "12abff");
        assert_eq!(Rgb::from_hex("#12ABFF"), Some(c));
        assert_eq!(Rgb::from_hex("xyz"), None);
    }
}
