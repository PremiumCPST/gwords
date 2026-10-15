//! Format-agnostic rich text document model.
//!
//! This is the contract shared by the file format readers/writers
//! (`crate::formats`) and the editor (`crate::editor::convert`).
//!
//! Conventions:
//! - A document is a flat list of blocks: paragraphs and tables.
//! - A table is a rectangular grid (every row has the same number of cells,
//!   at least 1x1). A cell contains paragraphs only (nested tables are
//!   flattened by readers); an empty cell has an empty `paragraphs` vector or
//!   a single empty paragraph (both are equivalent; see `Document::normalized`).
//! - Inside a run, `'\t'` is a tab and `'\u{2028}'` (LINE SEPARATOR) is a soft
//!   line break (ODT `<text:line-break/>`, DOCX `<w:br/>`). Runs never contain `'\n'`.
//! - An empty paragraph has an empty `runs` vector (or runs with empty text).
//! - Hyperlinks are a character attribute (`CharStyle::link`): consecutive runs
//!   with the same link form one hyperlink.

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Document {
    pub metadata: Metadata,
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    Paragraph(Paragraph),
    Table(Table),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Table {
    pub rows: Vec<TableRow>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TableRow {
    pub cells: Vec<TableCell>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TableCell {
    pub paragraphs: Vec<Paragraph>,
}

impl Table {
    /// Empty table of `rows` x `cols` (each at least 1).
    pub fn new(rows: usize, cols: usize) -> Table {
        let row = TableRow { cells: vec![TableCell::default(); cols.max(1)] };
        Table { rows: vec![row; rows.max(1)] }
    }

    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    pub fn column_count(&self) -> usize {
        self.rows.iter().map(|r| r.cells.len()).max().unwrap_or(0)
    }

    /// Pads short rows with empty cells so the grid is rectangular, and
    /// guarantees at least one row and one column.
    pub fn make_rectangular(&mut self) {
        if self.rows.is_empty() {
            self.rows.push(TableRow::default());
        }
        let cols = self.column_count().max(1);
        for row in &mut self.rows {
            row.cells.resize(cols, TableCell::default());
        }
    }
}

impl TableCell {
    pub fn plain_text(&self) -> String {
        self.paragraphs.iter().map(Paragraph::plain_text).collect::<Vec<_>>().join("\n")
    }
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
    /// Hyperlink target (URL), if this text is part of a link.
    pub link: Option<String>,
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

impl Block {
    pub fn as_paragraph(&self) -> Option<&Paragraph> {
        match self {
            Block::Paragraph(p) => Some(p),
            Block::Table(_) => None,
        }
    }

    pub fn as_table(&self) -> Option<&Table> {
        match self {
            Block::Table(t) => Some(t),
            Block::Paragraph(_) => None,
        }
    }
}

impl From<Paragraph> for Block {
    fn from(p: Paragraph) -> Block {
        Block::Paragraph(p)
    }
}

impl From<Table> for Block {
    fn from(t: Table) -> Block {
        Block::Table(t)
    }
}

impl Document {
    /// Builds a document of plain paragraphs, one per line.
    pub fn from_plain_text(text: &str) -> Document {
        let blocks = text
            .split('\n')
            .map(|line| {
                let line = line.trim_end_matches('\r');
                Block::Paragraph(Paragraph {
                    runs: if line.is_empty() { vec![] } else { vec![Run::plain(line)] },
                    ..Default::default()
                })
            })
            .collect();
        Document { metadata: Metadata::default(), blocks }
    }

    /// Top-level paragraphs only (tables skipped).
    pub fn paragraphs(&self) -> impl Iterator<Item = &Paragraph> {
        self.blocks.iter().filter_map(Block::as_paragraph)
    }

    pub fn tables(&self) -> impl Iterator<Item = &Table> {
        self.blocks.iter().filter_map(Block::as_table)
    }

    /// Plain text; table cells are separated by tabs and rows by newlines.
    pub fn plain_text(&self) -> String {
        self.blocks
            .iter()
            .map(|b| match b {
                Block::Paragraph(p) => p.plain_text(),
                Block::Table(t) => t
                    .rows
                    .iter()
                    .map(|r| r.cells.iter().map(|c| c.plain_text().replace('\n', " ")).collect::<Vec<_>>().join("\t"))
                    .collect::<Vec<_>>()
                    .join("\n"),
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Canonical form used to compare documents: adjacent runs with equal
    /// style merged, empty runs dropped, tables made rectangular, and empty
    /// cells represented by an empty `paragraphs` vector.
    pub fn normalized(&self) -> Document {
        fn norm_para(p: &Paragraph) -> Paragraph {
            let mut runs: Vec<Run> = Vec::new();
            for r in p.runs.iter().filter(|r| !r.text.is_empty()) {
                match runs.last_mut() {
                    Some(last) if last.style == r.style => last.text.push_str(&r.text),
                    _ => runs.push(r.clone()),
                }
            }
            Paragraph { runs, ..p.clone() }
        }
        let blocks = self
            .blocks
            .iter()
            .map(|b| match b {
                Block::Paragraph(p) => Block::Paragraph(norm_para(p)),
                Block::Table(t) => {
                    let mut t = t.clone();
                    t.make_rectangular();
                    for cell in t.rows.iter_mut().flat_map(|r| r.cells.iter_mut()) {
                        cell.paragraphs = cell.paragraphs.iter().map(norm_para).collect();
                        if cell.paragraphs.len() == 1 && cell.paragraphs[0] == Paragraph::default() {
                            cell.paragraphs.clear();
                        }
                    }
                    Block::Table(t)
                }
            })
            .collect();
        Document { metadata: self.metadata.clone(), blocks }
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

    #[test]
    fn tables_and_normalization() {
        let mut t = Table::new(2, 3);
        assert_eq!((t.row_count(), t.column_count()), (2, 3));
        t.rows[1].cells.truncate(1);
        t.rows[0].cells[0].paragraphs.push(Paragraph { runs: vec![Run::plain("a"), Run::plain("b")], ..Default::default() });
        t.rows[0].cells[1].paragraphs.push(Paragraph::default());
        let doc = Document { metadata: Metadata::default(), blocks: vec![Block::Table(t)] };
        let n = doc.normalized();
        let t = n.tables().next().unwrap();
        assert_eq!(t.rows[1].cells.len(), 3);
        assert_eq!(t.rows[0].cells[0].paragraphs[0].runs, vec![Run::plain("ab")]);
        assert!(t.rows[0].cells[1].paragraphs.is_empty());
        assert_eq!(doc.plain_text(), "ab\t\t\n");
    }
}
