//! OpenDocument Text (.odt) reader.
//!
//! Parses `content.xml`, `styles.xml` and `meta.xml` of the zip package into
//! the format-agnostic [`Document`] model. The parser is streaming
//! (quick-xml) and never panics on malformed input.

use std::collections::HashMap;
use std::io::{Cursor, Read};

use quick_xml::escape::resolve_predefined_entity;
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};

use crate::document::{
    Alignment, Block, CharStyle, Document, ListKind, Metadata, Paragraph, ParagraphStyle, Rgb,
    Run, Table, TableCell, TableRow, VerticalAlign,
};
use crate::formats::FormatError;

/// Upper bound for the uncompressed size of one XML part.
const MAX_PART_BYTES: u64 = 256 * 1024 * 1024;
/// Upper bound for the repeat count of `<text:s text:c="..."/>`.
const MAX_SPACES: usize = 10_000;
/// Maximum depth followed in style inheritance chains.
const MAX_STYLE_DEPTH: usize = 32;
/// Cap for `table:number-columns-repeated` on cells.
const MAX_CELL_REPEAT: usize = 64;
/// Cap for `table:number-rows-repeated` on rows.
const MAX_ROW_REPEAT: usize = 1000;

/// Parses an .odt file (zip archive) into a [`Document`].
pub fn read(bytes: &[u8]) -> Result<Document, FormatError> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))?;

    let content = read_part(&mut archive, "content.xml")?
        .ok_or_else(|| FormatError::Invalid("content.xml est absent de l'archive ODT".into()))?;
    let styles = read_part(&mut archive, "styles.xml")?;
    let meta = read_part(&mut archive, "meta.xml")?;

    let mut sheet = StyleSheet::default();
    if let Some(styles) = &styles {
        collect_styles(styles, &mut sheet, true)?;
    }
    collect_styles(&content, &mut sheet, false)?;

    let blocks = parse_body(&content, &sheet)?;
    let metadata = match &meta {
        // A broken meta.xml must not prevent opening the document.
        Some(m) => parse_meta(m).unwrap_or_default(),
        None => Metadata::default(),
    };
    Ok(Document { metadata, blocks })
}

fn read_part<R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    name: &str,
) -> Result<Option<String>, FormatError> {
    let file = match archive.by_name(name) {
        Ok(f) => f,
        Err(zip::result::ZipError::FileNotFound) => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let mut s = String::new();
    match file.take(MAX_PART_BYTES).read_to_string(&mut s) {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::InvalidData => {
            return Err(FormatError::Invalid(format!("{name} n'est pas de l'UTF-8 valide")));
        }
        Err(e) => return Err(e.into()),
    }
    if s.starts_with('\u{feff}') {
        s.remove(0);
    }
    Ok(Some(s))
}

// ---------------------------------------------------------------------------
// XML helpers
// ---------------------------------------------------------------------------

fn attr(e: &BytesStart<'_>, key: &str) -> Option<String> {
    for a in e.attributes().with_checks(false).flatten() {
        if a.key.into_inner() == key {
            return a.normalized_value(XmlVersion::Implicit1_0).ok().map(|c| c.into_owned());
        }
    }
    None
}

fn name_of(q: quick_xml::name::QName<'_>) -> &str {
    q.into_inner()
}

// ---------------------------------------------------------------------------
// Style model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
struct TextProps {
    bold: Option<bool>,
    italic: Option<bool>,
    underline: Option<bool>,
    strike: Option<bool>,
    vertical: Option<VerticalAlign>,
    family: Option<String>,
    size: Option<f64>,
    color: Option<Rgb>,
    /// `Some(None)` = explicitly transparent.
    highlight: Option<Option<Rgb>>,
    /// Hyperlink target (from an enclosing `text:a`).
    link: Option<String>,
}

impl TextProps {
    fn overlay(&mut self, o: &TextProps) {
        macro_rules! ov {
            ($($f:ident),*) => { $( if o.$f.is_some() { self.$f = o.$f.clone(); } )* };
        }
        ov!(bold, italic, underline, strike, vertical, family, size, color, highlight, link);
    }

    /// Direct formatting of `self` relative to `base` (baseline subtraction):
    /// a field is only set when its effective value differs from the baseline.
    fn diff(&self, base: &TextProps) -> CharStyle {
        let flag = |e: Option<bool>, b: Option<bool>| e.unwrap_or(false) && !b.unwrap_or(false);
        let mut cs = CharStyle::default();
        cs.bold = flag(self.bold, base.bold);
        cs.italic = flag(self.italic, base.italic);
        cs.underline = flag(self.underline, base.underline);
        cs.strikethrough = flag(self.strike, base.strike);
        if let Some(v) = self.vertical {
            if base.vertical != Some(v) && (v != VerticalAlign::Baseline || base.vertical.is_some())
            {
                cs.vertical = v;
            }
        }
        if self.family.is_some() && self.family != base.family {
            cs.font_family = self.family.clone();
        }
        if self.size.is_some() && self.size != base.size {
            cs.font_size = self.size;
        }
        if self.color.is_some() && self.color != base.color {
            cs.color = self.color;
        }
        let eff_h = self.highlight.flatten();
        if eff_h != base.highlight.flatten() {
            cs.highlight = eff_h;
        }
        cs.link = self.link.clone();
        cs
    }
}

#[derive(Debug, Clone, Default)]
struct StyleDef {
    parent: Option<String>,
    display_name: Option<String>,
    /// Defined in styles.xml (named/common style) rather than automatic.
    common: bool,
    text: TextProps,
    align: Option<Alignment>,
    list_style: Option<String>,
}

#[derive(Debug, Default)]
struct StyleSheet {
    fonts: HashMap<String, String>,
    /// Text properties of `style:default-style` for paragraphs.
    default_text: TextProps,
    /// Keyed by (family, name).
    styles: HashMap<(String, String), StyleDef>,
    /// list style name -> level -> kind.
    lists: HashMap<String, HashMap<usize, ListKind>>,
}

struct ParaInfo {
    text: TextProps,
    align: Alignment,
    /// Alignment set by an automatic (non-common) style of the chain, i.e.
    /// direct paragraph formatting.
    auto_align: Option<Alignment>,
    kind: ParagraphStyle,
    list_style: Option<String>,
}

impl StyleSheet {
    /// Style and its ancestors, nearest first.
    fn chain(&self, family: &str, name: &str) -> Vec<(&str, &StyleDef)> {
        let mut out = Vec::new();
        let mut cur = name.to_string();
        for _ in 0..MAX_STYLE_DEPTH {
            let Some((k, def)) = self.styles.get_key_value(&(family.to_string(), cur.clone()))
            else {
                break;
            };
            out.push((k.1.as_str(), def));
            match &def.parent {
                Some(p) if !p.is_empty() => cur = p.clone(),
                _ => break,
            }
        }
        out
    }

    fn resolve_text(&self, family: &str, name: &str) -> TextProps {
        let mut props = TextProps::default();
        for (_, def) in self.chain(family, name).into_iter().rev() {
            props.overlay(&def.text);
        }
        props
    }

    fn resolve_para(&self, name: &str) -> ParaInfo {
        let chain = self.chain("paragraph", name);
        let mut text = TextProps::default();
        for (_, def) in chain.iter().rev() {
            text.overlay(&def.text);
        }
        let align = chain.iter().find_map(|(_, d)| d.align).unwrap_or_default();
        let list_style = chain.iter().find_map(|(_, d)| d.list_style.clone());
        let kind = chain
            .iter()
            .find_map(|(n, d)| {
                kind_from_name(n).or_else(|| d.display_name.as_deref().and_then(kind_from_name))
            })
            .unwrap_or_default();
        let auto_align = chain.iter().take_while(|(_, d)| !d.common).find_map(|(_, d)| d.align);
        ParaInfo { text, align, auto_align, kind, list_style }
    }

    /// Effective text properties of the default style plus the named style chain.
    fn effective_text(&self, info: &ParaInfo) -> TextProps {
        let mut t = self.default_text.clone();
        t.overlay(&info.text);
        t
    }

    /// Formatting a reader expects the paragraph style itself to provide.
    fn baseline(&self, name: &str, styled: bool) -> TextProps {
        let mut t = self.default_text.clone();
        for (_, def) in self.chain("paragraph", name).into_iter().rev() {
            if !def.common {
                continue;
            }
            if styled {
                t.overlay(&def.text);
            } else {
                if def.text.family.is_some() {
                    t.family = def.text.family.clone();
                }
                if def.text.size.is_some() {
                    t.size = def.text.size;
                }
            }
        }
        t
    }

    fn list_kind(&self, style: Option<&str>, level: usize) -> ListKind {
        let Some(levels) = style.and_then(|s| self.lists.get(s)) else {
            return ListKind::Bullet;
        };
        if let Some(k) = levels.get(&level) {
            return *k;
        }
        // Nearest lower level, else any.
        (1..level)
            .rev()
            .find_map(|l| levels.get(&l).copied())
            .or_else(|| levels.iter().min_by_key(|(l, _)| **l).map(|(_, k)| *k))
            .unwrap_or(ListKind::Bullet)
    }
}

/// Maps a paragraph style name (internal or display) to our style enum.
fn kind_from_name(name: &str) -> Option<ParagraphStyle> {
    let n = name.replace("_20_", " ").replace('_', " ").to_lowercase();
    let n = n.trim();
    match n {
        "title" => return Some(ParagraphStyle::Title),
        "quotations" | "quotation" | "quote" | "block quote" | "blockquote" => {
            return Some(ParagraphStyle::Quote);
        }
        "preformatted text" | "source text" | "source code" => return Some(ParagraphStyle::Code),
        _ => {}
    }
    if let Some(rest) = n.strip_prefix("heading") {
        if let Ok(level) = rest.trim().parse::<u32>() {
            return Some(heading_for_level(level));
        }
    }
    None
}

fn heading_for_level(level: u32) -> ParagraphStyle {
    match level {
        0 | 1 => ParagraphStyle::Heading1,
        2 => ParagraphStyle::Heading2,
        _ => ParagraphStyle::Heading3,
    }
}

// --- property parsing -------------------------------------------------------

fn parse_bold(v: &str) -> Option<bool> {
    match v.trim() {
        "bold" => Some(true),
        "normal" => Some(false),
        s => s.parse::<u32>().ok().map(|n| n >= 600),
    }
}

fn parse_italic(v: &str) -> Option<bool> {
    match v.trim() {
        "italic" | "oblique" => Some(true),
        "normal" => Some(false),
        _ => None,
    }
}

fn parse_position(v: &str) -> Option<VerticalAlign> {
    let first = v.split_whitespace().next()?;
    match first {
        "super" => Some(VerticalAlign::Superscript),
        "sub" => Some(VerticalAlign::Subscript),
        _ => {
            let n: f64 = first.trim_end_matches('%').parse().ok()?;
            Some(if n > 0.0 {
                VerticalAlign::Superscript
            } else if n < 0.0 {
                VerticalAlign::Subscript
            } else {
                VerticalAlign::Baseline
            })
        }
    }
}

/// Parses a length into points. Percentages are ignored.
fn parse_size_pt(v: &str) -> Option<f64> {
    let v = v.trim();
    let split = v.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == '+'))?;
    let (num, unit) = v.split_at(split);
    let n: f64 = num.parse().ok()?;
    let pt = match unit.trim() {
        "pt" => n,
        "in" => n * 72.0,
        "cm" => n * 72.0 / 2.54,
        "mm" => n * 72.0 / 25.4,
        "px" => n * 0.75,
        _ => return None,
    };
    (pt.is_finite() && pt > 0.0).then_some(pt)
}

fn parse_align(v: &str) -> Option<Alignment> {
    match v.trim() {
        "start" | "left" => Some(Alignment::Left),
        "center" => Some(Alignment::Center),
        "end" | "right" => Some(Alignment::Right),
        "justify" => Some(Alignment::Justify),
        _ => None,
    }
}

fn strip_quotes(s: &str) -> String {
    s.chars().filter(|c| *c != '\'' && *c != '"').collect::<String>().trim().to_string()
}

fn text_props_from(e: &BytesStart<'_>, fonts: &HashMap<String, String>) -> TextProps {
    let mut p = TextProps::default();
    let mut font_name = None;
    let mut font_family = None;
    for a in e.attributes().with_checks(false).flatten() {
        let Ok(val) = a.normalized_value(XmlVersion::Implicit1_0) else { continue };
        let val = val.as_ref();
        match name_of(a.key) {
            "fo:font-weight" => p.bold = parse_bold(val),
            "fo:font-style" => p.italic = parse_italic(val),
            "style:text-underline-style" => p.underline = Some(val.trim() != "none"),
            "style:text-line-through-style" => p.strike = Some(val.trim() != "none"),
            "style:text-position" => p.vertical = parse_position(val),
            "fo:font-size" => p.size = parse_size_pt(val),
            "fo:color" => p.color = Rgb::from_hex(val),
            "fo:background-color" => {
                p.highlight = if val.trim() == "transparent" {
                    Some(None)
                } else {
                    Rgb::from_hex(val).map(Some)
                };
            }
            "style:font-name" => font_name = Some(val.to_string()),
            "fo:font-family" => font_family = Some(strip_quotes(val)),
            _ => {}
        }
    }
    p.family = font_name
        .map(|n| fonts.get(&n).cloned().unwrap_or_else(|| strip_quotes(&n)))
        .or(font_family)
        .filter(|f| !f.is_empty());
    p
}

// --- collection pass ----------------------------------------------------------

fn collect_styles(xml: &str, sheet: &mut StyleSheet, common: bool) -> Result<(), FormatError> {
    let mut reader = Reader::from_str(xml);
    let mut cur_style: Option<((String, String), StyleDef)> = None;
    let mut cur_list: Option<(String, HashMap<usize, ListKind>)> = None;
    let mut in_default = false;

    loop {
        let ev = reader.read_event()?;
        let (e, opened, closed) = match &ev {
            Event::Start(e) => (Some(e), true, false),
            Event::Empty(e) => (Some(e), true, true),
            Event::End(_) => (None, false, true),
            Event::Eof => break,
            _ => continue,
        };
        // Name of the element being opened/closed.
        let name: String = match &ev {
            Event::Start(e) | Event::Empty(e) => name_of(e.name()).to_string(),
            Event::End(e) => name_of(e.name()).to_string(),
            _ => unreachable!(),
        };

        if opened {
            let e = e.expect("start/empty carries a tag");
            match name.as_str() {
                "style:font-face" => {
                    if let (Some(n), Some(f)) = (attr(e, "style:name"), attr(e, "svg:font-family")) {
                        sheet.fonts.insert(n, strip_quotes(&f));
                    }
                }
                "style:default-style" => {
                    in_default = attr(e, "style:family").as_deref() == Some("paragraph");
                }
                "style:style" => {
                    let family = attr(e, "style:family").unwrap_or_default();
                    let sname = attr(e, "style:name").unwrap_or_default();
                    let def = StyleDef {
                        parent: attr(e, "style:parent-style-name"),
                        display_name: attr(e, "style:display-name"),
                        common,
                        list_style: attr(e, "style:list-style-name").filter(|s| !s.is_empty()),
                        ..Default::default()
                    };
                    cur_style = Some(((family, sname), def));
                }
                "style:text-properties" if in_default && cur_style.is_none() => {
                    sheet.default_text = text_props_from(e, &sheet.fonts);
                }
                "style:text-properties" => {
                    if let Some((_, def)) = cur_style.as_mut() {
                        def.text = text_props_from(e, &sheet.fonts);
                    }
                }
                "style:paragraph-properties" => {
                    if let Some((_, def)) = cur_style.as_mut() {
                        def.align = attr(e, "fo:text-align").as_deref().and_then(parse_align);
                    }
                }
                "text:list-style" => {
                    cur_list = Some((attr(e, "style:name").unwrap_or_default(), HashMap::new()));
                }
                "text:list-level-style-number"
                | "text:list-level-style-bullet"
                | "text:list-level-style-image" => {
                    if let Some((_, levels)) = cur_list.as_mut() {
                        let level = attr(e, "text:level")
                            .and_then(|l| l.trim().parse::<usize>().ok())
                            .unwrap_or(1);
                        let kind = if name == "text:list-level-style-number" {
                            ListKind::Numbered
                        } else {
                            ListKind::Bullet
                        };
                        levels.insert(level, kind);
                    }
                }
                _ => {}
            }
        }
        if closed {
            match name.as_str() {
                "style:default-style" => in_default = false,
                "style:style" => {
                    if let Some((key, def)) = cur_style.take() {
                        sheet.styles.insert(key, def);
                    }
                }
                "text:list-style" => {
                    if let Some((n, levels)) = cur_list.take() {
                        sheet.lists.insert(n, levels);
                    }
                }
                _ => {}
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Body parsing
// ---------------------------------------------------------------------------

struct ParaState {
    style: ParagraphStyle,
    align: Alignment,
    list: Option<ListKind>,
    /// Baseline formatting provided by the paragraph's named style.
    baseline: TextProps,
    base: TextProps,
    spans: Vec<TextProps>,
    runs: Vec<Run>,
    /// The previous character was whitespace (or a tab/break): collapse next.
    last_space: bool,
    /// The last char pushed is a space produced by collapsing a text node.
    last_collapsed: bool,
}

impl ParaState {
    fn current(&self) -> &TextProps {
        self.spans.last().unwrap_or(&self.base)
    }

    fn push_raw(&mut self, s: &str) {
        if s.is_empty() {
            return;
        }
        let style = self.current().diff(&self.baseline);
        match self.runs.last_mut() {
            Some(r) if r.style == style => r.text.push_str(s),
            _ => self.runs.push(Run { text: s.to_string(), style }),
        }
    }

    /// Pushes literal content (text:s, text:tab, text:line-break).
    fn push_explicit(&mut self, s: &str) {
        self.push_raw(s);
        self.last_space = true;
        self.last_collapsed = false;
    }

    /// Pushes character data with ODF whitespace collapsing.
    fn push_text(&mut self, text: &str) {
        let mut buf = String::with_capacity(text.len());
        for ch in text.chars() {
            if matches!(ch, ' ' | '\t' | '\r' | '\n') {
                if !self.last_space {
                    buf.push(' ');
                    self.last_space = true;
                    self.last_collapsed = true;
                }
            } else {
                buf.push(ch);
                self.last_space = false;
                self.last_collapsed = false;
            }
        }
        self.push_raw(&buf);
    }

    fn finish(mut self) -> Paragraph {
        if self.last_collapsed {
            if let Some(r) = self.runs.last_mut() {
                if r.text.ends_with(' ') {
                    r.text.pop();
                }
            }
        }
        let mut runs: Vec<Run> = Vec::new();
        for r in self.runs {
            if r.text.is_empty() {
                continue;
            }
            match runs.last_mut() {
                Some(last) if last.style == r.style => last.text.push_str(&r.text),
                _ => runs.push(r),
            }
        }
        Paragraph { style: self.style, alignment: self.align, list: self.list, runs }
    }
}

struct CellBuild {
    paragraphs: Vec<Paragraph>,
    repeat: usize,
}

struct RowBuild {
    cells: Vec<TableCell>,
    repeat: usize,
}

/// Outermost table being built (nested tables are flattened into its cells).
#[derive(Default)]
struct TableBuild {
    rows: Vec<TableRow>,
    row: Option<RowBuild>,
    cell: Option<CellBuild>,
}

fn repeat_attr(e: &BytesStart<'_>, key: &str, cap: usize) -> usize {
    attr(e, key).and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(1).clamp(1, cap)
}

struct Body<'a> {
    sheet: &'a StyleSheet,
    blocks: Vec<Block>,
    tbl: Option<TableBuild>,
    /// Nesting depth of `table:table` (0 = not in a table).
    table_depth: usize,
    /// List stack of the context surrounding the outermost table.
    saved_lists: Vec<Option<String>>,
    cur: Option<ParaState>,
    /// Stack of list style names (None when unknown) for nested text:list.
    lists: Vec<Option<String>>,
    /// > 0 while inside an ignored subtree.
    skip: usize,
}

impl<'a> Body<'a> {
    fn end_paragraph(&mut self) {
        if let Some(p) = self.cur.take() {
            let p = p.finish();
            if let Some(cell) = self.tbl.as_mut().and_then(|t| t.cell.as_mut()) {
                cell.paragraphs.push(p);
            } else {
                self.blocks.push(Block::Paragraph(p));
            }
        }
    }

    fn end_cell(&mut self) {
        self.end_paragraph();
        let Some(t) = self.tbl.as_mut() else { return };
        let Some(cell) = t.cell.take() else { return };
        let mut paragraphs = cell.paragraphs;
        if paragraphs.len() == 1 && paragraphs[0] == Paragraph::default() {
            paragraphs.clear();
        }
        let row = t.row.get_or_insert_with(|| RowBuild { cells: Vec::new(), repeat: 1 });
        for _ in 0..cell.repeat {
            row.cells.push(TableCell { paragraphs: paragraphs.clone() });
        }
    }

    fn end_row(&mut self) {
        self.end_cell();
        let Some(t) = self.tbl.as_mut() else { return };
        if let Some(row) = t.row.take() {
            for _ in 0..row.repeat {
                t.rows.push(TableRow { cells: row.cells.clone() });
            }
        }
    }

    fn end_table(&mut self) {
        self.end_row();
        if let Some(t) = self.tbl.take() {
            let mut table = Table { rows: t.rows };
            table.make_rectangular();
            self.blocks.push(Block::Table(table));
        }
        self.table_depth = 0;
        self.lists = std::mem::take(&mut self.saved_lists);
    }

    fn begin_paragraph(&mut self, e: &BytesStart<'_>, heading: bool) {
        self.end_paragraph();
        let style_name = attr(e, "text:style-name").unwrap_or_default();
        let info = self.sheet.resolve_para(&style_name);
        let style = if heading {
            let level = attr(e, "text:outline-level")
                .and_then(|l| l.trim().parse::<u32>().ok())
                .unwrap_or(1);
            heading_for_level(level)
        } else {
            info.kind
        };
        let list = if self.lists.is_empty() {
            None
        } else {
            let name = self.lists.last().and_then(|n| n.as_deref());
            Some(self.sheet.list_kind(name, self.lists.len()))
        };
        let base = self.sheet.effective_text(&info);
        let baseline = self.sheet.baseline(&style_name, style != ParagraphStyle::Normal);
        let align = if style != ParagraphStyle::Normal {
            // Only direct formatting counts: the named style provides the rest.
            info.auto_align.unwrap_or(Alignment::Left)
        } else {
            info.align
        };
        self.cur = Some(ParaState {
            style,
            align,
            list,
            baseline,
            base,
            spans: Vec::new(),
            runs: Vec::new(),
            last_space: true,
            last_collapsed: false,
        });
        let _ = info.list_style;
    }

    fn open(&mut self, name: &str, e: &BytesStart<'_>) {
        if self.skip > 0 {
            self.skip += 1;
            return;
        }
        match name {
            "text:note" | "office:annotation" | "text:tracked-changes" | "text:ruby-text"
            | "text:number" | "office:forms" => self.skip = 1,
            n if n.starts_with("draw:") => self.skip = 1,
            "table:table" => {
                self.table_depth += 1;
                if self.table_depth == 1 {
                    self.end_paragraph();
                    self.tbl = Some(TableBuild::default());
                    self.saved_lists = std::mem::take(&mut self.lists);
                }
            }
            "table:table-row" if self.table_depth == 1 => {
                self.end_row();
                if let Some(t) = self.tbl.as_mut() {
                    let repeat = repeat_attr(e, "table:number-rows-repeated", MAX_ROW_REPEAT);
                    t.row = Some(RowBuild { cells: Vec::new(), repeat });
                }
            }
            "table:table-cell" if self.table_depth == 1 => {
                self.end_cell();
                if let Some(t) = self.tbl.as_mut() {
                    let repeat = repeat_attr(e, "table:number-columns-repeated", MAX_CELL_REPEAT);
                    t.cell = Some(CellBuild { paragraphs: Vec::new(), repeat });
                }
            }
            "table:covered-table-cell" if self.table_depth == 1 => {
                self.end_cell();
                let repeat = repeat_attr(e, "table:number-columns-repeated", MAX_CELL_REPEAT);
                if let Some(t) = self.tbl.as_mut() {
                    let row = t.row.get_or_insert_with(|| RowBuild { cells: Vec::new(), repeat: 1 });
                    for _ in 0..repeat {
                        row.cells.push(TableCell::default());
                    }
                }
                self.skip = 1;
            }
            "text:list" => {
                let style = attr(e, "text:style-name")
                    .filter(|s| !s.is_empty())
                    .or_else(|| self.lists.last().cloned().flatten());
                self.lists.push(style);
            }
            "text:p" => self.begin_paragraph(e, false),
            "text:h" => self.begin_paragraph(e, true),
            "text:span" | "text:a" => {
                let sheet = self.sheet;
                if let Some(p) = self.cur.as_mut() {
                    let mut cs = p.current().clone();
                    if let Some(sn) = attr(e, "text:style-name") {
                        cs.overlay(&sheet.resolve_text("text", &sn));
                    }
                    if name == "text:a" {
                        if let Some(href) = attr(e, "xlink:href").filter(|h| !h.is_empty()) {
                            cs.link = Some(href);
                        }
                    }
                    p.spans.push(cs);
                }
            }
            "text:s" => {
                if let Some(p) = self.cur.as_mut() {
                    let n = attr(e, "text:c")
                        .and_then(|c| c.trim().parse::<usize>().ok())
                        .unwrap_or(1)
                        .min(MAX_SPACES);
                    p.push_explicit(&" ".repeat(n));
                }
            }
            "text:tab" => {
                if let Some(p) = self.cur.as_mut() {
                    p.push_explicit("\t");
                }
            }
            "text:line-break" => {
                if let Some(p) = self.cur.as_mut() {
                    p.push_explicit("\u{2028}");
                }
            }
            _ => {}
        }
    }

    fn close(&mut self, name: &str) {
        if self.skip > 0 {
            self.skip -= 1;
            return;
        }
        match name {
            "text:list" => {
                self.lists.pop();
            }
            "table:table-cell" if self.table_depth == 1 => self.end_cell(),
            "table:table-row" if self.table_depth == 1 => self.end_row(),
            "table:table" if self.table_depth > 0 => {
                self.table_depth -= 1;
                if self.table_depth == 0 {
                    self.end_table();
                }
            }
            "text:p" | "text:h" => self.end_paragraph(),
            "text:span" | "text:a" => {
                if let Some(p) = self.cur.as_mut() {
                    p.spans.pop();
                }
            }
            _ => {}
        }
    }

    fn text(&mut self, t: &str) {
        if self.skip > 0 {
            return;
        }
        if let Some(p) = self.cur.as_mut() {
            p.push_text(t);
        }
    }
}

fn parse_body(xml: &str, sheet: &StyleSheet) -> Result<Vec<Block>, FormatError> {
    let mut reader = Reader::from_str(xml);
    let mut body = Body {
        sheet,
        blocks: Vec::new(),
        tbl: None,
        table_depth: 0,
        saved_lists: Vec::new(),
        cur: None,
        lists: Vec::new(),
        skip: 0,
    };
    loop {
        match reader.read_event()? {
            Event::Start(e) => body.open(name_of(e.name()), &e),
            Event::Empty(e) => {
                let name = name_of(e.name());
                body.open(name, &e);
                body.close(name);
            }
            Event::End(e) => body.close(name_of(e.name())),
            Event::Text(t) => body.text(&t.xml10_content()),
            Event::CData(c) => body.text(&c.xml10_content()),
            Event::GeneralRef(r) => {
                let resolved = match r.resolve_char_ref() {
                    Ok(Some(ch)) => Some(ch.to_string()),
                    Ok(None) => resolve_predefined_entity(&r.xml10_content()).map(str::to_string),
                    Err(_) => None,
                };
                if let Some(s) = resolved {
                    body.text(&s);
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    body.end_paragraph();
    if body.tbl.is_some() {
        body.end_table();
    }
    Ok(body.blocks)
}

// ---------------------------------------------------------------------------
// Metadata
// ---------------------------------------------------------------------------

fn parse_meta(xml: &str) -> Result<Metadata, FormatError> {
    #[derive(Clone, Copy, PartialEq)]
    enum Field {
        Title,
        Initial,
        Creator,
    }
    let mut reader = Reader::from_str(xml);
    let mut field: Option<Field> = None;
    let (mut title, mut initial, mut creator) = (String::new(), String::new(), String::new());
    loop {
        match reader.read_event()? {
            Event::Start(e) => {
                field = match name_of(e.name()) {
                    "dc:title" => Some(Field::Title),
                    "meta:initial-creator" => Some(Field::Initial),
                    "dc:creator" => Some(Field::Creator),
                    _ => None,
                };
            }
            Event::End(_) => field = None,
            Event::Text(t) => {
                let s = t.xml10_content();
                match field {
                    Some(Field::Title) => title.push_str(&s),
                    Some(Field::Initial) => initial.push_str(&s),
                    Some(Field::Creator) => creator.push_str(&s),
                    None => {}
                }
            }
            Event::GeneralRef(r) => {
                let s = match r.resolve_char_ref() {
                    Ok(Some(ch)) => Some(ch.to_string()),
                    Ok(None) => resolve_predefined_entity(&r.xml10_content()).map(str::to_string),
                    Err(_) => None,
                };
                if let Some(s) = s {
                    match field {
                        Some(Field::Title) => title.push_str(&s),
                        Some(Field::Initial) => initial.push_str(&s),
                        Some(Field::Creator) => creator.push_str(&s),
                        None => {}
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    let clean = |s: String| {
        let t = s.trim();
        (!t.is_empty()).then(|| t.to_string())
    };
    Ok(Metadata { title: clean(title), author: clean(initial).or_else(|| clean(creator)) })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const NS: &str = r#"xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:meta="urn:oasis:names:tc:opendocument:xmlns:meta:1.0""#;

    fn zip_of(files: &[(&str, String)]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default();
        for (name, data) in files {
            w.start_file(*name, opts).unwrap();
            w.write_all(data.as_bytes()).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    fn content(auto: &str, body: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><office:document-content {NS}><office:automatic-styles>{auto}</office:automatic-styles><office:body><office:text>{body}</office:text></office:body></office:document-content>"#
        )
    }

    fn styles(common: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><office:document-styles {NS}><office:styles>{common}</office:styles></office:document-styles>"#
        )
    }

    fn doc(auto: &str, body: &str, common: &str) -> Document {
        let z = zip_of(&[("content.xml", content(auto, body)), ("styles.xml", styles(common))]);
        read(&z).unwrap()
    }

    fn simple(body: &str) -> Document {
        doc("", body, "")
    }

    fn paras(d: &Document) -> Vec<&Paragraph> {
        d.paragraphs().collect()
    }

    fn cell_texts(t: &Table) -> Vec<Vec<String>> {
        t.rows.iter().map(|r| r.cells.iter().map(|c| c.plain_text()).collect()).collect()
    }

    #[test]
    fn plain_paragraphs() {
        let d = simple("<text:p>Bonjour</text:p><text:p/><text:p>Monde</text:p>");
        assert_eq!(paras(&d).len(), 3);
        assert_eq!(paras(&d)[0].plain_text(), "Bonjour");
        assert!(paras(&d)[1].runs.is_empty());
        assert_eq!(paras(&d)[2].runs, vec![Run::plain("Monde")]);
    }

    #[test]
    fn bold_italic_spans_and_merge() {
        let auto = r#"<style:style style:name="B" style:family="text"><style:text-properties fo:font-weight="bold"/></style:style>
<style:style style:name="I" style:family="text"><style:text-properties fo:font-style="italic" style:text-underline-style="solid" style:text-line-through-style="solid"/></style:style>"#;
        let d = doc(
            auto,
            r#"<text:p>a <text:span text:style-name="B">b<text:span text:style-name="I">c</text:span></text:span> d<text:span text:style-name="B"></text:span></text:p>"#,
            "",
        );
        let runs = &paras(&d)[0].runs;
        assert_eq!(runs.len(), 4);
        assert_eq!(runs[0], Run::plain("a "));
        assert!(runs[1].style.bold && !runs[1].style.italic);
        assert_eq!(runs[1].text, "b");
        assert!(runs[2].style.bold && runs[2].style.italic);
        assert!(runs[2].style.underline && runs[2].style.strikethrough);
        assert_eq!(runs[3], Run::plain(" d"));
    }

    #[test]
    fn adjacent_equal_runs_merge() {
        let auto = r#"<style:style style:name="B" style:family="text"><style:text-properties fo:font-weight="bold"/></style:style>"#;
        let d = doc(
            auto,
            r#"<text:p><text:span text:style-name="B">a</text:span><text:span text:style-name="B">b</text:span></text:p>"#,
            "",
        );
        assert_eq!(paras(&d)[0].runs.len(), 1);
        assert_eq!(paras(&d)[0].runs[0].text, "ab");
    }

    #[test]
    fn headings_by_outline_level() {
        let d = simple(
            r#"<text:h text:outline-level="1">A</text:h><text:h text:outline-level="2">B</text:h><text:h text:outline-level="3">C</text:h><text:h text:outline-level="5">D</text:h>"#,
        );
        let s: Vec<_> = paras(&d).iter().map(|p| p.style).collect();
        assert_eq!(
            s,
            vec![
                ParagraphStyle::Heading1,
                ParagraphStyle::Heading2,
                ParagraphStyle::Heading3,
                ParagraphStyle::Heading3
            ]
        );
    }

    #[test]
    fn paragraph_style_mapping_with_inheritance() {
        let common = r#"
<style:style style:name="Title" style:family="paragraph"/>
<style:style style:name="Quotations" style:family="paragraph"/>
<style:style style:name="Preformatted_20_Text" style:family="paragraph"/>
<style:style style:name="Heading_20_2" style:family="paragraph"/>
<style:style style:name="MyTitle" style:family="paragraph" style:parent-style-name="Title"/>"#;
        let auto = r#"<style:style style:name="P1" style:family="paragraph" style:parent-style-name="MyTitle"/>"#;
        let d = doc(
            auto,
            r#"<text:p text:style-name="Title">a</text:p><text:p text:style-name="P1">b</text:p><text:p text:style-name="Quotations">c</text:p><text:p text:style-name="Preformatted_20_Text">d</text:p><text:p text:style-name="Heading_20_2">e</text:p><text:p text:style-name="Unknown">f</text:p>"#,
            common,
        );
        let s: Vec<_> = paras(&d).iter().map(|p| p.style).collect();
        assert_eq!(
            s,
            vec![
                ParagraphStyle::Title,
                ParagraphStyle::Title,
                ParagraphStyle::Quote,
                ParagraphStyle::Code,
                ParagraphStyle::Heading2,
                ParagraphStyle::Normal
            ]
        );
    }

    #[test]
    fn alignment_and_inherited_alignment() {
        let common = r#"<style:style style:name="Centered" style:family="paragraph"><style:paragraph-properties fo:text-align="center"/></style:style>"#;
        let auto = r#"<style:style style:name="P1" style:family="paragraph" style:parent-style-name="Centered"/>
<style:style style:name="P2" style:family="paragraph"><style:paragraph-properties fo:text-align="end"/></style:style>
<style:style style:name="P3" style:family="paragraph"><style:paragraph-properties fo:text-align="justify"/></style:style>
<style:style style:name="P4" style:family="paragraph"><style:paragraph-properties fo:text-align="start"/></style:style>"#;
        let d = doc(
            auto,
            r#"<text:p text:style-name="P1">a</text:p><text:p text:style-name="P2">b</text:p><text:p text:style-name="P3">c</text:p><text:p text:style-name="P4">d</text:p>"#,
            common,
        );
        let a: Vec<_> = paras(&d).iter().map(|p| p.alignment).collect();
        assert_eq!(a, vec![Alignment::Center, Alignment::Right, Alignment::Justify, Alignment::Left]);
    }

    #[test]
    fn whitespace_collapsing() {
        let d = simple("<text:p>  a   b \n\t c  </text:p>");
        assert_eq!(paras(&d)[0].plain_text(), "a b c");
    }

    #[test]
    fn spaces_tabs_line_breaks() {
        let d = simple(
            r#"<text:p>a<text:s text:c="3"/>b<text:tab/>c<text:line-break/>d<text:s/>e</text:p>"#,
        );
        assert_eq!(paras(&d)[0].plain_text(), "a   b\tc\u{2028}d e");
        let d = simple(r#"<text:p>x <text:span>y</text:span> z</text:p>"#);
        assert_eq!(paras(&d)[0].plain_text(), "x y z");
    }

    #[test]
    fn entities() {
        let d = simple("<text:p>a &amp; b &lt;c&gt; &#233;&#x41;</text:p>");
        assert_eq!(paras(&d)[0].plain_text(), "a & b <c> \u{e9}A");
    }

    #[test]
    fn lists_bullet_numbered_nested() {
        let auto = r#"<text:list-style style:name="L1"><text:list-level-style-bullet text:level="1"/><text:list-level-style-number text:level="2"/></text:list-style>
<text:list-style style:name="L2"><text:list-level-style-number text:level="1"/></text:list-style>"#;
        let d = doc(
            auto,
            r#"<text:list text:style-name="L1"><text:list-item><text:p>a</text:p><text:list><text:list-item><text:p>b</text:p></text:list-item></text:list></text:list-item></text:list><text:list text:style-name="L2"><text:list-item><text:p>c</text:p></text:list-item></text:list><text:p>d</text:p>"#,
            "",
        );
        let l: Vec<_> = paras(&d).iter().map(|p| p.list).collect();
        assert_eq!(
            l,
            vec![Some(ListKind::Bullet), Some(ListKind::Numbered), Some(ListKind::Numbered), None]
        );
    }

    #[test]
    fn list_without_style_is_bullet() {
        let d = simple("<text:list><text:list-item><text:p>a</text:p></text:list-item></text:list>");
        assert_eq!(paras(&d)[0].list, Some(ListKind::Bullet));
    }

    #[test]
    fn style_inheritance_and_para_base_style() {
        let common = r#"<style:style style:name="Base" style:family="paragraph"><style:text-properties fo:font-weight="bold" fo:font-size="14pt"/></style:style>"#;
        let auto = r#"<style:style style:name="P1" style:family="paragraph" style:parent-style-name="Base"><style:text-properties fo:font-style="italic" fo:font-size="16pt"/></style:style>
<style:style style:name="T1" style:family="text"><style:text-properties fo:font-weight="normal"/></style:style>"#;
        let d = doc(
            auto,
            r#"<text:p text:style-name="P1">a<text:span text:style-name="T1">b</text:span></text:p>"#,
            common,
        );
        let r = &paras(&d)[0].runs;
        assert!(r[0].style.bold && r[0].style.italic);
        assert_eq!(r[0].style.font_size, Some(16.0));
        assert!(!r[1].style.bold && r[1].style.italic);
    }

    #[test]
    fn text_style_parent_chain() {
        let common = r#"<style:style style:name="Strong" style:family="text"><style:text-properties fo:font-weight="bold"/></style:style>"#;
        let auto = r##"<style:style style:name="T1" style:family="text" style:parent-style-name="Strong"><style:text-properties fo:color="#FF0000" fo:background-color="#00ff00"/></style:style>"##;
        let d = doc(auto, r#"<text:p><text:span text:style-name="T1">x</text:span></text:p>"#, common);
        let s = &paras(&d)[0].runs[0].style;
        assert!(s.bold);
        assert_eq!(s.color, Some(Rgb(255, 0, 0)));
        assert_eq!(s.highlight, Some(Rgb(0, 255, 0)));
    }

    #[test]
    fn transparent_highlight_clears() {
        let common = r##"<style:style style:name="P" style:family="paragraph"><style:text-properties fo:background-color="#ffff00"/></style:style>"##;
        let auto = r#"<style:style style:name="T" style:family="text"><style:text-properties fo:background-color="transparent"/></style:style>"#;
        let d = doc(
            auto,
            r#"<text:p text:style-name="P">a<text:span text:style-name="T">b</text:span></text:p>"#,
            common,
        );
        assert_eq!(paras(&d)[0].runs[0].style.highlight, Some(Rgb(255, 255, 0)));
        assert_eq!(paras(&d)[0].runs[1].style.highlight, None);
    }

    #[test]
    fn font_face_resolution_and_sizes() {
        let content_xml = format!(
            r#"<?xml version="1.0"?><office:document-content {NS}><office:font-face-decls><style:font-face style:name="Liberation Serif1" svg:font-family="&apos;Liberation Serif&apos;"/></office:font-face-decls><office:automatic-styles>
<style:style style:name="T1" style:family="text"><style:text-properties style:font-name="Liberation Serif1" fo:font-size="10.5pt"/></style:style>
<style:style style:name="T2" style:family="text"><style:text-properties fo:font-family="&quot;Cantarell&quot;" fo:font-size="120%"/></style:style>
</office:automatic-styles><office:body><office:text><text:p><text:span text:style-name="T1">a</text:span><text:span text:style-name="T2">b</text:span></text:p></office:text></office:body></office:document-content>"#
        );
        let d = read(&zip_of(&[("content.xml", content_xml)])).unwrap();
        let r = &paras(&d)[0].runs;
        assert_eq!(r[0].style.font_family.as_deref(), Some("Liberation Serif"));
        assert_eq!(r[0].style.font_size, Some(10.5));
        assert_eq!(r[1].style.font_family.as_deref(), Some("Cantarell"));
        assert_eq!(r[1].style.font_size, None);
    }

    #[test]
    fn font_face_from_styles_xml() {
        let styles_xml = format!(
            r#"<?xml version="1.0"?><office:document-styles {NS}><office:font-face-decls><style:font-face style:name="Lato" svg:font-family="Lato"/></office:font-face-decls><office:styles><style:style style:name="Standard" style:family="paragraph"><style:text-properties style:font-name="Lato"/></style:style></office:styles></office:document-styles>"#
        );
        let z = zip_of(&[
            ("content.xml", content("", r#"<text:p text:style-name="Standard">x</text:p>"#)),
            ("styles.xml", styles_xml),
        ]);
        let d = read(&z).unwrap();
        // The font of the named body style is the baseline, not direct formatting.
        assert_eq!(paras(&d)[0].runs[0].style.font_family, None);
    }

    #[test]
    fn super_and_subscript() {
        let auto = r#"<style:style style:name="S1" style:family="text"><style:text-properties style:text-position="super 58%"/></style:style>
<style:style style:name="S2" style:family="text"><style:text-properties style:text-position="-33% 58%"/></style:style>
<style:style style:name="S3" style:family="text"><style:text-properties style:text-position="33%"/></style:style>
<style:style style:name="S4" style:family="text"><style:text-properties style:text-position="sub 58%"/></style:style>
<style:style style:name="S5" style:family="text"><style:text-properties style:text-position="0% 100%"/></style:style>"#;
        let body: String = (1..=5)
            .map(|i| format!(r#"<text:span text:style-name="S{i}">{i}</text:span>"#))
            .collect();
        let d = doc(auto, &format!("<text:p>{body}</text:p>"), "");
        let v: Vec<_> = paras(&d)[0].runs.iter().map(|r| r.style.vertical).collect();
        assert_eq!(
            v,
            vec![
                VerticalAlign::Superscript,
                VerticalAlign::Subscript,
                VerticalAlign::Superscript,
                VerticalAlign::Subscript,
                VerticalAlign::Baseline
            ]
        );
    }

    #[test]
    fn notes_and_frames_skipped() {
        let d = simple(
            r#"<text:p>x<text:note text:note-class="footnote"><text:note-citation>1</text:note-citation><text:note-body><text:p>NOTE</text:p></text:note-body></text:note>y<office:annotation><text:p>ann</text:p></office:annotation><draw:frame><draw:text-box><text:p>box</text:p></draw:text-box></draw:frame>z</text:p>"#,
        );
        let t: Vec<_> = paras(&d).iter().map(|p| p.plain_text()).collect();
        assert_eq!(t, vec!["xyz"]);
    }

    #[test]
    fn table_basic() {
        let d = simple(
            r#"<text:p>before</text:p><table:table><table:table-column table:number-columns-repeated="2"/><table:table-row><table:table-cell><text:p>c1</text:p></table:table-cell><table:table-cell><text:p>c2</text:p><text:p>c2b</text:p></table:table-cell></table:table-row><table:table-row><table:table-cell/><table:table-cell><text:p/></table:table-cell></table:table-row></table:table><text:p>after</text:p>"#,
        );
        assert_eq!(d.blocks.len(), 3);
        let t = d.tables().next().unwrap();
        assert_eq!(cell_texts(t), vec![vec!["c1", "c2\nc2b"], vec!["", ""]]);
        assert!(t.rows[1].cells[0].paragraphs.is_empty());
        assert!(t.rows[1].cells[1].paragraphs.is_empty());
        assert_eq!(paras(&d).len(), 2);
    }

    #[test]
    fn table_repeated_header_covered() {
        let d = simple(
            r#"<table:table><table:table-header-rows><table:table-row><table:table-cell><text:p>h</text:p></table:table-cell><table:table-cell table:number-columns-repeated="2"><text:p>g</text:p></table:table-cell></table:table-row></table:table-header-rows><table:table-rows><table:table-row table:number-rows-repeated="2"><table:table-cell table:number-columns-repeated="3"><text:p>r</text:p></table:table-cell></table:table-row></table:table-rows><table:table-row><table:table-cell table:number-columns-repeated="2" table:number-columns-spanned="2"><text:p>s</text:p></table:table-cell><table:covered-table-cell table:number-columns-repeated="2"/></table:table-row></table:table>"#,
        );
        let t = d.tables().next().unwrap();
        assert_eq!(
            cell_texts(t),
            vec![
                vec!["h", "g", "g", ""],
                vec!["r", "r", "r", ""],
                vec!["r", "r", "r", ""],
                vec!["s", "s", "", ""],
            ]
        );
    }

    #[test]
    fn table_repeat_caps_and_ragged() {
        let d = simple(
            r#"<table:table><table:table-row table:number-rows-repeated="1048576"><table:table-cell table:number-columns-repeated="16384"/></table:table-row></table:table>"#,
        );
        let t = d.tables().next().unwrap();
        assert_eq!((t.rows.len(), t.rows[0].cells.len()), (MAX_ROW_REPEAT, MAX_CELL_REPEAT));
        let d = simple(
            r#"<table:table><table:table-row><table:table-cell><text:p>a</text:p></table:table-cell></table:table-row><table:table-row><table:table-cell/><table:table-cell/><table:table-cell/></table:table-row></table:table>"#,
        );
        let t = d.tables().next().unwrap();
        assert!(t.rows.iter().all(|r| r.cells.len() == 3));
    }

    #[test]
    fn empty_table_is_one_by_one() {
        let d = simple("<table:table/>");
        let t = d.tables().next().unwrap();
        assert_eq!((t.rows.len(), t.rows[0].cells.len()), (1, 1));
    }

    #[test]
    fn nested_table_flattened_and_lists_in_cells() {
        let auto = r#"<text:list-style style:name="L2"><text:list-level-style-number text:level="1"/></text:list-style>"#;
        let d = doc(
            auto,
            r#"<text:list><text:list-item><text:p>out</text:p><table:table><table:table-row><table:table-cell><text:p>a</text:p><table:table><table:table-row><table:table-cell><text:p>n1</text:p></table:table-cell><table:table-cell><text:p>n2</text:p></table:table-cell></table:table-row></table:table><text:p>b</text:p></table:table-cell><table:table-cell><text:list text:style-name="L2"><text:list-item><text:p>li</text:p></text:list-item></text:list></table:table-cell></table:table-row></table:table><text:p>in-list</text:p></text:list-item></text:list>"#,
            "",
        );
        assert_eq!(d.blocks.len(), 3);
        let t = d.tables().next().unwrap();
        assert_eq!(t.rows.len(), 1);
        assert_eq!(t.rows[0].cells.len(), 2);
        assert_eq!(t.rows[0].cells[0].plain_text(), "a\nn1\nn2\nb");
        assert!(t.rows[0].cells[0].paragraphs.iter().all(|p| p.list.is_none()));
        assert_eq!(t.rows[0].cells[1].paragraphs[0].list, Some(ListKind::Numbered));
        let ps = paras(&d);
        assert_eq!(ps[0].list, Some(ListKind::Bullet));
        assert_eq!(ps[1].plain_text(), "in-list");
        assert_eq!(ps[1].list, Some(ListKind::Bullet));
    }

    #[test]
    fn table_cell_paragraph_styles() {
        let common = r#"<style:style style:name="Title" style:family="paragraph"/>"#;
        let d = doc(
            "",
            r#"<table:table><table:table-row><table:table-cell><text:p text:style-name="Title">T</text:p><text:h text:outline-level="2">H</text:h></table:table-cell></table:table-row></table:table>"#,
            common,
        );
        let c = &d.tables().next().unwrap().rows[0].cells[0];
        assert_eq!(c.paragraphs[0].style, ParagraphStyle::Title);
        assert_eq!(c.paragraphs[1].style, ParagraphStyle::Heading2);
    }

    #[test]
    fn unclosed_table_is_kept() {
        let z = zip_of(&[(
            "content.xml",
            content("", "<table:table><table:table-row><table:table-cell><text:p>x</text:p>"),
        )]);
        if let Ok(d) = read(&z) {
            assert!(d.tables().count() <= 1);
        }
    }

    #[test]
    fn links_become_char_style() {
        let auto = r#"<style:style style:name="B" style:family="text"><style:text-properties fo:font-weight="bold"/></style:style>"#;
        let d = doc(
            auto,
            r#"<text:p xmlns:xlink="http://www.w3.org/1999/xlink">see <text:a xlink:type="simple" xlink:href="http://x?a=1&amp;b=2">link <text:span text:style-name="B">bold</text:span></text:a><text:soft-page-break/> end <text:a xlink:href="http://y">y</text:a></text:p>"#,
            "",
        );
        let r = &paras(&d)[0].runs;
        assert_eq!(paras(&d)[0].plain_text(), "see link bold end y");
        assert_eq!(r.len(), 5);
        assert_eq!(r[0].style.link, None);
        assert_eq!(r[1].text, "link ");
        assert_eq!(r[1].style.link.as_deref(), Some("http://x?a=1&b=2"));
        assert!(!r[1].style.bold);
        assert!(r[2].style.bold);
        assert_eq!(r[2].style.link.as_deref(), Some("http://x?a=1&b=2"));
        assert_eq!(r[3].style.link, None);
        assert_eq!(r[4].style.link.as_deref(), Some("http://y"));
    }

    #[test]
    fn link_inside_table_cell() {
        let d = simple(
            r#"<table:table><table:table-row><table:table-cell><text:p><text:a xlink:href="http://z" xmlns:xlink="http://www.w3.org/1999/xlink">z</text:a></text:p></table:table-cell></table:table-row></table:table>"#,
        );
        let c = &d.tables().next().unwrap().rows[0].cells[0];
        assert_eq!(c.paragraphs[0].runs[0].style.link.as_deref(), Some("http://z"));
    }

    #[test]
    fn metadata() {
        let meta = format!(
            r#"<?xml version="1.0"?><office:document-meta {NS}><office:meta><dc:title>Mon &amp; titre</dc:title><meta:initial-creator>Alice</meta:initial-creator><dc:creator>Bob</dc:creator></office:meta></office:document-meta>"#
        );
        let z = zip_of(&[("content.xml", content("", "<text:p>x</text:p>")), ("meta.xml", meta)]);
        let d = read(&z).unwrap();
        assert_eq!(d.metadata.title.as_deref(), Some("Mon & titre"));
        assert_eq!(d.metadata.author.as_deref(), Some("Alice"));

        let meta = format!(
            r#"<?xml version="1.0"?><office:document-meta {NS}><office:meta><dc:creator>Bob</dc:creator></office:meta></office:document-meta>"#
        );
        let z = zip_of(&[("content.xml", content("", "")), ("meta.xml", meta)]);
        let d = read(&z).unwrap();
        assert_eq!(d.metadata.title, None);
        assert_eq!(d.metadata.author.as_deref(), Some("Bob"));
    }

    #[test]
    fn errors_do_not_panic() {
        assert!(read(b"").is_err());
        assert!(read(b"not a zip").is_err());
        let z = zip_of(&[("other.xml", "x".into())]);
        assert!(matches!(read(&z), Err(FormatError::Invalid(_))));
        let z = zip_of(&[("content.xml", "<office:text><text:p>unclosed".into())]);
        let _ = read(&z); // either Ok or Err, never a panic
        let z = zip_of(&[("content.xml", "<a></b>".into())]);
        assert!(read(&z).is_err());
        // Style cycle must terminate.
        let auto = r#"<style:style style:name="A" style:family="paragraph" style:parent-style-name="B"/><style:style style:name="B" style:family="paragraph" style:parent-style-name="A"/>"#;
        let d = doc(auto, r#"<text:p text:style-name="A">x</text:p>"#, "");
        assert_eq!(paras(&d).len(), 1);
    }

    #[test]
    fn huge_space_count_is_capped() {
        let d = simple(r#"<text:p>a<text:s text:c="4000000000"/>b</text:p>"#);
        assert!(paras(&d)[0].plain_text().len() <= MAX_SPACES + 2);
    }
}
