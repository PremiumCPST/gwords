//! DOCX (Office Open XML) reader: zip + quick-xml streaming into the document model.

use std::collections::{HashMap, HashSet};
use std::io::{Cursor, Read};

use quick_xml::Reader;
use quick_xml::events::Event;
use zip::ZipArchive;

use crate::document::{
    Alignment, Block, CharStyle, Document, ListKind, Metadata, Paragraph, ParagraphStyle, Rgb, Run,
    Table, TableCell, TableRow, VerticalAlign,
};
use crate::formats::FormatError;

const MAX_PART_SIZE: u64 = 256 * 1024 * 1024;

/// Parses a `.docx` file into a [`Document`].
pub fn read(bytes: &[u8]) -> Result<Document, FormatError> {
    let mut archive = ZipArchive::new(Cursor::new(bytes))?;

    let main_path = find_main_part(&mut archive)?.unwrap_or_else(|| "word/document.xml".to_string());
    let main = read_part(&mut archive, &main_path)?
        .ok_or_else(|| FormatError::Invalid("partie principale du document introuvable".into()))?;
    let dir = match main_path.rfind('/') {
        Some(i) => main_path[..=i].to_string(),
        None => String::new(),
    };

    let styles_bytes = match read_part(&mut archive, &format!("{dir}styles.xml"))? {
        Some(b) => Some(b),
        None => read_part(&mut archive, "word/styles.xml")?,
    };
    let numbering_bytes = match read_part(&mut archive, &format!("{dir}numbering.xml"))? {
        Some(b) => Some(b),
        None => read_part(&mut archive, "word/numbering.xml")?,
    };

    let styles = match styles_bytes {
        Some(b) => parse_styles(&tokenize(&b)?),
        None => Styles::default(),
    };
    let numbering = match numbering_bytes {
        Some(b) => Some(parse_numbering(&tokenize(&b)?)),
        None => None,
    };

    let rels_path = {
        let (d, f) = match main_path.rfind('/') {
            Some(i) => (&main_path[..=i], &main_path[i + 1..]),
            None => ("", main_path.as_str()),
        };
        format!("{d}_rels/{f}.rels")
    };
    let rels = match read_part(&mut archive, &rels_path)? {
        Some(b) => parse_hyperlink_rels(&tokenize(&b).unwrap_or_default()),
        None => HashMap::new(),
    };

    let blocks = parse_document(&tokenize(&main)?, &styles, numbering.as_ref(), &rels);

    let metadata = match read_part(&mut archive, "docProps/core.xml")? {
        Some(b) => parse_core(&tokenize(&b).unwrap_or_default()),
        None => Metadata::default(),
    };

    Ok(Document { metadata, blocks })
}

// ---------------------------------------------------------------------------
// Zip helpers
// ---------------------------------------------------------------------------

fn read_part<R: Read + std::io::Seek>(
    archive: &mut ZipArchive<R>,
    name: &str,
) -> Result<Option<Vec<u8>>, FormatError> {
    let file = match archive.by_name(name) {
        Ok(f) => f,
        Err(zip::result::ZipError::FileNotFound) => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let mut buf = Vec::new();
    file.take(MAX_PART_SIZE).read_to_end(&mut buf)?;
    Ok(Some(buf))
}

fn find_main_part<R: Read + std::io::Seek>(
    archive: &mut ZipArchive<R>,
) -> Result<Option<String>, FormatError> {
    let Some(rels) = read_part(archive, "_rels/.rels")? else {
        return Ok(None);
    };
    let events = tokenize(&rels)?;
    for ev in &events {
        if let Ev::Start(name, attrs) = ev {
            if name == "Relationship" {
                let ty = attr(attrs, "Type").unwrap_or("");
                if ty.ends_with("/officeDocument") {
                    if let Some(target) = attr(attrs, "Target") {
                        let t = target.trim_start_matches('/').to_string();
                        if !t.is_empty() && archive.by_name(&t).is_ok() {
                            return Ok(Some(t));
                        }
                    }
                }
            }
        }
    }
    Ok(None)
}

// ---------------------------------------------------------------------------
// Tokenizer
// ---------------------------------------------------------------------------

type Attrs = Vec<(String, String)>;

#[derive(Debug, Clone)]
enum Ev {
    Start(String, Attrs),
    End(String),
    Text(String),
}

fn attr<'a>(attrs: &'a Attrs, key: &str) -> Option<&'a str> {
    attrs.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

fn clean_text(s: &str) -> String {
    s.chars()
        .filter(|c| *c != '\r')
        .map(|c| if c == '\n' { ' ' } else { c })
        .collect()
}

fn tokenize(bytes: &[u8]) -> Result<Vec<Ev>, FormatError> {
    let mut reader = Reader::from_reader(bytes);
    reader.config_mut().expand_empty_elements = true;
    let mut out = Vec::new();
    loop {
        let ev = reader.read_event().map_err(|e| FormatError::Xml(e.to_string()))?;
        match ev {
            Event::Eof => break,
            Event::Start(e) => {
                let name = e.local_name().as_ref().to_string();
                let mut attrs = Attrs::new();
                for a in e.attributes().flatten() {
                    let key = a.key.local_name().as_ref().to_string();
                    let val = a.normalized_value(quick_xml::XmlVersion::Implicit1_0).map(|v| v.into_owned()).unwrap_or_default();
                    attrs.push((key, val));
                }
                out.push(Ev::Start(name, attrs));
            }
            Event::End(e) => {
                out.push(Ev::End(e.local_name().as_ref().to_string()));
            }
            Event::Text(t) => out.push(Ev::Text(clean_text(&t.xml10_content()))),
            Event::CData(c) => out.push(Ev::Text(clean_text(c.as_ref()))),
            Event::GeneralRef(r) => {
                let s: &str = &r;
                let text = match s {
                    "lt" => Some('<'),
                    "gt" => Some('>'),
                    "amp" => Some('&'),
                    "quot" => Some('"'),
                    "apos" => Some('\''),
                    _ => r.resolve_char_ref().ok().flatten(),
                };
                if let Some(c) = text {
                    out.push(Ev::Text(c.to_string()));
                }
            }
            _ => {}
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Property parsing
// ---------------------------------------------------------------------------

fn on_off(attrs: &Attrs) -> bool {
    match attr(attrs, "val") {
        None => true,
        Some(v) => !matches!(v.to_ascii_lowercase().as_str(), "0" | "false" | "off"),
    }
}

fn highlight_color(name: &str) -> Option<Rgb> {
    let hex = match name.to_ascii_lowercase().as_str() {
        "yellow" => "ffff00",
        "green" => "00ff00",
        "cyan" => "00ffff",
        "magenta" => "ff00ff",
        "blue" => "0000ff",
        "red" => "ff0000",
        "darkblue" => "000080",
        "darkcyan" => "008080",
        "darkgreen" => "008000",
        "darkmagenta" => "800080",
        "darkred" => "800000",
        "darkyellow" => "808000",
        "darkgray" => "808080",
        "lightgray" => "c0c0c0",
        "black" => "000000",
        "white" => "ffffff",
        _ => return None,
    };
    Rgb::from_hex(hex)
}

/// Run properties as written in a single `w:rPr` (every field is an override).
#[derive(Debug, Clone, Default)]
struct RProps {
    bold: Option<bool>,
    italic: Option<bool>,
    underline: Option<bool>,
    strike: Option<bool>,
    vertical: Option<VerticalAlign>,
    font: Option<String>,
    size: Option<f64>,
    color: Option<Rgb>,
    highlight: Option<Option<Rgb>>,
    shading: Option<Rgb>,
    style: Option<String>,
}

impl RProps {
    fn apply(&self, cs: &mut CharStyle) {
        if let Some(v) = self.bold {
            cs.bold = v;
        }
        if let Some(v) = self.italic {
            cs.italic = v;
        }
        if let Some(v) = self.underline {
            cs.underline = v;
        }
        if let Some(v) = self.strike {
            cs.strikethrough = v;
        }
        if let Some(v) = self.vertical {
            cs.vertical = v;
        }
        if let Some(v) = &self.font {
            cs.font_family = Some(v.clone());
        }
        if let Some(v) = self.size {
            cs.font_size = Some(v);
        }
        if let Some(v) = self.color {
            cs.color = Some(v);
        }
        if let Some(v) = self.shading {
            cs.highlight = Some(v);
        }
        if let Some(v) = self.highlight {
            cs.highlight = v;
        }
    }
}

/// Handles a direct child element of `w:rPr`.
fn parse_rpr_child(name: &str, attrs: &Attrs, p: &mut RProps) {
    match name {
        "b" => p.bold = Some(on_off(attrs)),
        "i" => p.italic = Some(on_off(attrs)),
        "strike" | "dstrike" => p.strike = Some(on_off(attrs)),
        "u" => {
            p.underline = Some(match attr(attrs, "val") {
                None => true,
                Some(v) => !matches!(v.to_ascii_lowercase().as_str(), "none" | "0" | "false" | "off"),
            })
        }
        "vertAlign" => {
            p.vertical = match attr(attrs, "val") {
                Some("superscript") => Some(VerticalAlign::Superscript),
                Some("subscript") => Some(VerticalAlign::Subscript),
                Some("baseline") => Some(VerticalAlign::Baseline),
                _ => None,
            }
        }
        "sz" => {
            if let Some(v) = attr(attrs, "val").and_then(|v| v.trim().parse::<f64>().ok()) {
                if v > 0.0 && v.is_finite() {
                    p.size = Some(v / 2.0);
                }
            }
        }
        "rFonts" => {
            let f = attr(attrs, "ascii").or_else(|| attr(attrs, "hAnsi"));
            if let Some(f) = f.filter(|f| !f.is_empty()) {
                p.font = Some(f.to_string());
            }
        }
        "color" => {
            if let Some(v) = attr(attrs, "val") {
                if !v.eq_ignore_ascii_case("auto") {
                    if let Some(c) = Rgb::from_hex(v) {
                        p.color = Some(c);
                    }
                }
            }
        }
        "highlight" => {
            if let Some(v) = attr(attrs, "val") {
                if v.eq_ignore_ascii_case("none") {
                    p.highlight = Some(None);
                } else if let Some(c) = highlight_color(v) {
                    p.highlight = Some(Some(c));
                }
            }
        }
        "shd" => {
            if let Some(v) = attr(attrs, "fill") {
                if !v.eq_ignore_ascii_case("auto") {
                    if let Some(c) = Rgb::from_hex(v) {
                        p.shading = Some(c);
                    }
                }
            }
        }
        "rStyle" => p.style = attr(attrs, "val").map(str::to_string),
        _ => {}
    }
}

/// Paragraph properties of a `w:pPr`.
#[derive(Debug, Clone, Default)]
struct PProps {
    style: Option<String>,
    jc: Option<Alignment>,
    outline: Option<u32>,
    num_id: Option<String>,
    ilvl: Option<u32>,
}

fn parse_ppr_child(name: &str, parent: &str, attrs: &Attrs, p: &mut PProps) {
    match (parent, name) {
        ("pPr", "pStyle") => p.style = attr(attrs, "val").map(str::to_string),
        ("pPr", "jc") => {
            p.jc = match attr(attrs, "val").map(|v| v.to_ascii_lowercase()).as_deref() {
                Some("left") | Some("start") => Some(Alignment::Left),
                Some("center") => Some(Alignment::Center),
                Some("right") | Some("end") => Some(Alignment::Right),
                Some("both") | Some("distribute") | Some("justify") => Some(Alignment::Justify),
                _ => None,
            }
        }
        ("pPr", "outlineLvl") => p.outline = attr(attrs, "val").and_then(|v| v.trim().parse().ok()),
        ("numPr", "numId") => p.num_id = attr(attrs, "val").map(|v| v.trim().to_string()),
        ("numPr", "ilvl") => p.ilvl = attr(attrs, "val").and_then(|v| v.trim().parse().ok()),
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Styles
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
struct Style {
    name: String,
    based_on: Option<String>,
    rpr: RProps,
    ppr: PProps,
}

#[derive(Debug, Default)]
struct Styles {
    defaults: RProps,
    map: HashMap<String, Style>,
    default_paragraph: Option<String>,
}

fn parse_styles(events: &[Ev]) -> Styles {
    let mut st = Styles::default();
    let mut stack: Vec<&str> = Vec::new();
    let mut cur: Option<(String, Style)> = None;
    let mut cur_is_paragraph = false;
    let mut cur_is_default = false;
    for ev in events {
        match ev {
            Ev::Start(name, attrs) => {
                let parent = stack.last().copied().unwrap_or("");
                let in_change = stack.iter().any(|s| s.ends_with("Change"));
                if !in_change {
                    match name.as_str() {
                        "style" => {
                            cur = Some((attr(attrs, "styleId").unwrap_or("").to_string(), Style::default()));
                            cur_is_paragraph = attr(attrs, "type").unwrap_or("paragraph") == "paragraph";
                            cur_is_default = attr(attrs, "default").is_some_and(|v| matches!(v, "1" | "true" | "on"));
                        }
                        "name" if parent == "style" => {
                            if let Some((_, s)) = cur.as_mut() {
                                s.name = attr(attrs, "val").unwrap_or("").to_string();
                            }
                        }
                        "basedOn" if parent == "style" => {
                            if let Some((_, s)) = cur.as_mut() {
                                s.based_on = attr(attrs, "val").map(str::to_string);
                            }
                        }
                        _ if parent == "rPr" && !stack.contains(&"pPr") => {
                            if let Some((_, s)) = cur.as_mut() {
                                parse_rpr_child(name, attrs, &mut s.rpr);
                            } else if stack.contains(&"docDefaults") {
                                parse_rpr_child(name, attrs, &mut st.defaults);
                            }
                        }
                        _ => {
                            if let Some((_, s)) = cur.as_mut() {
                                if stack.contains(&"pPr") {
                                    parse_ppr_child(name, parent, attrs, &mut s.ppr);
                                }
                            }
                        }
                    }
                }
                stack.push(name.as_str());
            }
            Ev::End(name) => {
                stack.pop();
                if name == "style" {
                    if let Some((id, s)) = cur.take() {
                        if !id.is_empty() {
                            if cur_is_paragraph && cur_is_default && st.default_paragraph.is_none() {
                                st.default_paragraph = Some(id.clone());
                            }
                            st.map.insert(id, s);
                        }
                    }
                }
            }
            Ev::Text(_) => {}
        }
    }
    st
}

fn normalized_name(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace() && *c != '-' && *c != '_').flat_map(char::to_lowercase).collect()
}

fn map_style_name(s: &str) -> Option<ParagraphStyle> {
    let n = normalized_name(s);
    match n.as_str() {
        "title" | "titre" => return Some(ParagraphStyle::Title),
        "quote" | "intensequote" | "citation" | "citationintense" => return Some(ParagraphStyle::Quote),
        "htmlpreformatted" | "code" | "sourcecode" => return Some(ParagraphStyle::Code),
        _ => {}
    }
    for prefix in ["heading", "titre"] {
        if let Some(rest) = n.strip_prefix(prefix) {
            if let Ok(level) = rest.parse::<u32>() {
                return match level {
                    1 => Some(ParagraphStyle::Heading1),
                    2 => Some(ParagraphStyle::Heading2),
                    3..=9 => Some(ParagraphStyle::Heading3),
                    _ => None,
                };
            }
        }
    }
    None
}

impl Styles {
    /// Style then its ancestors (derived first). Cycle safe.
    fn chain(&self, id: &str) -> Vec<(&str, &Style)> {
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        let mut cur = Some(id);
        while let Some(c) = cur {
            if !seen.insert(c.to_string()) {
                break;
            }
            let Some((k, s)) = self.map.get_key_value(c) else { break };
            out.push((k.as_str(), s));
            cur = s.based_on.as_deref();
            if out.len() > 32 {
                break;
            }
        }
        out
    }

    fn apply_style_rpr(&self, id: &str, cs: &mut CharStyle) {
        for (_, s) in self.chain(id).into_iter().rev() {
            s.rpr.apply(cs);
        }
    }

    /// Base character style of a paragraph: docDefaults + paragraph style chain.
    fn base_char_style(&self, pstyle: Option<&str>) -> CharStyle {
        let mut cs = CharStyle::default();
        self.defaults.apply(&mut cs);
        if let Some(id) = pstyle.or(self.default_paragraph.as_deref()) {
            self.apply_style_rpr(id, &mut cs);
        }
        cs
    }
}

// ---------------------------------------------------------------------------
// Numbering
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
struct Numbering {
    /// abstractNumId -> (ilvl -> numFmt)
    abstracts: HashMap<String, HashMap<u32, String>>,
    /// numId -> abstractNumId
    nums: HashMap<String, String>,
}

fn parse_numbering(events: &[Ev]) -> Numbering {
    let mut n = Numbering::default();
    let mut stack: Vec<&str> = Vec::new();
    let mut cur_abs = String::new();
    let mut cur_lvl: Option<u32> = None;
    let mut cur_num = String::new();
    for ev in events {
        match ev {
            Ev::Start(name, attrs) => {
                let parent = stack.last().copied().unwrap_or("");
                match name.as_str() {
                    "abstractNum" => {
                        cur_abs = attr(attrs, "abstractNumId").unwrap_or("").to_string();
                        n.abstracts.entry(cur_abs.clone()).or_default();
                    }
                    "lvl" if parent == "abstractNum" => {
                        cur_lvl = attr(attrs, "ilvl").and_then(|v| v.trim().parse().ok());
                    }
                    "numFmt" if parent == "lvl" => {
                        if let (Some(l), Some(f)) = (cur_lvl, attr(attrs, "val")) {
                            n.abstracts.entry(cur_abs.clone()).or_default().insert(l, f.to_string());
                        }
                    }
                    "num" if parent == "numbering" => {
                        cur_num = attr(attrs, "numId").unwrap_or("").to_string();
                    }
                    "abstractNumId" if parent == "num" => {
                        if let Some(v) = attr(attrs, "val") {
                            n.nums.insert(cur_num.clone(), v.to_string());
                        }
                    }
                    _ => {}
                }
                stack.push(name.as_str());
            }
            Ev::End(name) => {
                stack.pop();
                if name == "lvl" {
                    cur_lvl = None;
                }
            }
            Ev::Text(_) => {}
        }
    }
    n
}

fn list_kind(numbering: Option<&Numbering>, num_id: &str, ilvl: u32) -> Option<ListKind> {
    if num_id.is_empty() || num_id == "0" {
        return None;
    }
    let Some(n) = numbering else { return Some(ListKind::Bullet) };
    let Some(levels) = n.nums.get(num_id).and_then(|a| n.abstracts.get(a)) else {
        return Some(ListKind::Bullet);
    };
    match levels.get(&ilvl).or_else(|| levels.get(&0)) {
        Some(f) if f == "bullet" => Some(ListKind::Bullet),
        Some(f) if f == "none" => Some(ListKind::Bullet),
        Some(_) => Some(ListKind::Numbered),
        None => Some(ListKind::Bullet),
    }
}

// ---------------------------------------------------------------------------
// Relationships and hyperlink field codes
// ---------------------------------------------------------------------------

/// Relationship id -> target, for hyperlink relationships of the main part.
fn parse_hyperlink_rels(events: &[Ev]) -> HashMap<String, String> {
    let mut m = HashMap::new();
    for ev in events {
        if let Ev::Start(name, attrs) = ev {
            if name == "Relationship" {
                let ty = attr(attrs, "Type").unwrap_or("");
                let external = attr(attrs, "TargetMode").is_some_and(|v| v.eq_ignore_ascii_case("External"));
                if ty.ends_with("/hyperlink") || external {
                    if let (Some(id), Some(t)) = (attr(attrs, "Id"), attr(attrs, "Target")) {
                        m.insert(id.to_string(), t.to_string());
                    }
                }
            }
        }
    }
    m
}

/// Splits a field instruction into words, honouring double quotes.
fn instr_words(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut has = false;
    for c in s.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                has = true;
            }
            c if c.is_whitespace() && !quoted => {
                if has {
                    out.push(std::mem::take(&mut cur));
                    has = false;
                }
            }
            c => {
                cur.push(c);
                has = true;
            }
        }
    }
    if has {
        out.push(cur);
    }
    out
}

/// Link target of a `HYPERLINK` field instruction, if it is one.
fn field_link(instr: &str) -> Option<String> {
    let words = instr_words(instr);
    let mut it = words.iter();
    if !it.next()?.eq_ignore_ascii_case("HYPERLINK") {
        return None;
    }
    let mut url: Option<String> = None;
    let mut anchor: Option<String> = None;
    while let Some(w) = it.next() {
        if w.starts_with('\\') {
            match w.to_ascii_lowercase().as_str() {
                "\\l" => anchor = it.next().cloned(),
                "\\o" | "\\t" => {
                    it.next();
                }
                _ => {}
            }
        } else if url.is_none() {
            url = Some(w.clone());
        }
    }
    match (url, anchor) {
        (Some(u), Some(a)) if !a.is_empty() => Some(format!("{u}#{a}")),
        (Some(u), _) if !u.is_empty() => Some(u),
        (_, Some(a)) if !a.is_empty() => Some(format!("#{a}")),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Core properties
// ---------------------------------------------------------------------------

fn parse_core(events: &[Ev]) -> Metadata {
    let mut m = Metadata::default();
    let mut current: Option<&str> = None;
    let mut buf = String::new();
    for ev in events {
        match ev {
            Ev::Start(name, _) => {
                if name == "title" || name == "creator" {
                    current = Some(name.as_str());
                    buf.clear();
                }
            }
            Ev::Text(t) => {
                if current.is_some() {
                    buf.push_str(t);
                }
            }
            Ev::End(name) => {
                if current == Some(name.as_str()) {
                    let v = buf.trim().to_string();
                    if !v.is_empty() {
                        if name == "title" {
                            m.title = Some(v);
                        } else {
                            m.author = Some(v);
                        }
                    }
                    current = None;
                }
            }
        }
    }
    m
}

// ---------------------------------------------------------------------------
// Main document
// ---------------------------------------------------------------------------

const SKIPPED: &[&str] = &[
    "drawing", "pict", "object", "del", "moveFrom", "sectPr", "delText",
    "rPrChange", "pPrChange", "numberingChange", "sectPrChange", "tblPrChange",
];

#[derive(Default)]
struct ParaBuilder {
    ppr: PProps,
    pending: Vec<(String, RProps, Option<String>)>,
}

#[derive(Default)]
struct RunBuilder {
    rpr: RProps,
    text: String,
}

fn push_run(runs: &mut Vec<Run>, text: String, style: CharStyle) {
    if text.is_empty() {
        return;
    }
    if let Some(last) = runs.last_mut() {
        if last.style == style {
            last.text.push_str(&text);
            return;
        }
    }
    runs.push(Run { text, style });
}

fn base_full(styles: &Styles, style_id: Option<&str>) -> CharStyle {
    styles.base_char_style(style_id)
}

fn finish_paragraph(pb: ParaBuilder, styles: &Styles, numbering: Option<&Numbering>) -> Paragraph {
    let style_id = pb.ppr.style.as_deref().or(styles.default_paragraph.as_deref());
    let chain = style_id.map(|id| styles.chain(id)).unwrap_or_default();

    let mut pstyle = ParagraphStyle::Normal;
    for (id, s) in &chain {
        if let Some(m) = map_style_name(id).or_else(|| map_style_name(&s.name)) {
            pstyle = m;
            break;
        }
    }
    if pstyle == ParagraphStyle::Normal {
        // A directly referenced but undefined style may still be recognisable by id.
        if let Some(id) = pb.ppr.style.as_deref() {
            if chain.is_empty() {
                pstyle = map_style_name(id).unwrap_or(ParagraphStyle::Normal);
            }
        }
    }
    if pstyle == ParagraphStyle::Normal {
        let outline = pb.ppr.outline.or_else(|| chain.iter().find_map(|(_, s)| s.ppr.outline));
        pstyle = match outline {
            Some(0) => ParagraphStyle::Heading1,
            Some(1) => ParagraphStyle::Heading2,
            Some(2) => ParagraphStyle::Heading3,
            _ => ParagraphStyle::Normal,
        };
    }

    let style_jc = if pstyle == ParagraphStyle::Normal { chain.iter().find_map(|(_, s)| s.ppr.jc) } else { None };
    let alignment = pb.ppr.jc.or(style_jc).unwrap_or(Alignment::Left);

    let num_id = pb.ppr.num_id.clone().or_else(|| chain.iter().find_map(|(_, s)| s.ppr.num_id.clone()));
    let ilvl = pb.ppr.ilvl.or_else(|| chain.iter().find_map(|(_, s)| s.ppr.ilvl)).unwrap_or(0);
    let list = num_id.and_then(|id| list_kind(numbering, &id, ilvl));

    // Baseline subtraction: only what differs from the paragraph's style is direct formatting.
    let mut base = styles.base_char_style(style_id);
    if pstyle == ParagraphStyle::Normal {
        // Normal has no visual baseline in the editor: keep style-level emphasis as direct formatting.
        let mut d = CharStyle::default();
        styles.defaults.apply(&mut d);
        base.bold = d.bold;
        base.italic = d.italic;
        base.underline = d.underline;
        base.strikethrough = d.strikethrough;
        base.color = d.color;
    }
    let mut runs = Vec::new();
    for (text, rpr, link) in pb.pending {
        let mut eff = base_full(styles, style_id);
        if let Some(rs) = rpr.style.as_deref() {
            // The character style of a link (usually "Hyperlink") must not become direct formatting.
            let (color, underline) = (eff.color, eff.underline);
            styles.apply_style_rpr(rs, &mut eff);
            if link.is_some() {
                eff.color = color;
                eff.underline = underline;
            }
        }
        rpr.apply(&mut eff);
        let cs = CharStyle {
            bold: eff.bold && !base.bold,
            italic: eff.italic && !base.italic,
            underline: eff.underline && !base.underline,
            strikethrough: eff.strikethrough && !base.strikethrough,
            vertical: if eff.vertical != base.vertical { eff.vertical } else { VerticalAlign::Baseline },
            font_family: if eff.font_family != base.font_family { eff.font_family } else { None },
            font_size: if eff.font_size != base.font_size { eff.font_size } else { None },
            color: if eff.color != base.color { eff.color } else { None },
            highlight: if eff.highlight != base.highlight { eff.highlight } else { None },
            link,
        };
        push_run(&mut runs, text, cs);
    }

    Paragraph { style: pstyle, alignment, list, runs }
}

#[derive(Default)]
struct CellBuilder {
    paragraphs: Vec<Paragraph>,
    span: usize,
    vmerge_continue: bool,
}

#[derive(Default)]
struct Field {
    instr: String,
    link: Option<String>,
    in_result: bool,
}

fn parse_document(
    events: &[Ev],
    styles: &Styles,
    numbering: Option<&Numbering>,
    rels: &HashMap<String, String>,
) -> Vec<Block> {
    let mut out: Vec<Block> = Vec::new();
    let mut stack: Vec<&str> = Vec::new();
    let mut skip = 0usize;
    let mut para: Option<ParaBuilder> = None;
    let mut run: Option<RunBuilder> = None;
    let mut in_t = false;
    let mut in_instr = false;
    let mut hyper: Option<String> = None;
    let mut fields: Vec<Field> = Vec::new();
    // Tables: only the outermost one is built, nested content is flattened into its cell.
    let mut tbl_depth = 0usize;
    let mut table = Table::default();
    let mut row: Option<TableRow> = None;
    let mut cell: Option<CellBuilder> = None;

    for ev in events {
        match ev {
            Ev::Start(name, attrs) => {
                if skip > 0 {
                    skip += 1;
                    continue;
                }
                if SKIPPED.contains(&name.as_str()) {
                    skip = 1;
                    continue;
                }
                let parent = stack.last().copied().unwrap_or("");
                match name.as_str() {
                    "tbl" => {
                        tbl_depth += 1;
                        if tbl_depth == 1 {
                            table = Table::default();
                            row = None;
                            cell = None;
                        }
                    }
                    "tr" if tbl_depth == 1 && para.is_none() => row = Some(TableRow::default()),
                    "tc" if tbl_depth == 1 && para.is_none() => {
                        cell = Some(CellBuilder { span: 1, ..Default::default() })
                    }
                    "gridSpan" if tbl_depth == 1 && parent == "tcPr" => {
                        if let (Some(c), Some(n)) =
                            (cell.as_mut(), attr(attrs, "val").and_then(|v| v.trim().parse::<usize>().ok()))
                        {
                            c.span = n.clamp(1, 1000);
                        }
                    }
                    "vMerge" if tbl_depth == 1 && parent == "tcPr" => {
                        if let Some(c) = cell.as_mut() {
                            c.vmerge_continue = attr(attrs, "val") != Some("restart");
                        }
                    }
                    "hyperlink" => {
                        let target = attr(attrs, "id").and_then(|id| rels.get(id)).cloned();
                        let anchor = attr(attrs, "anchor").filter(|a| !a.is_empty());
                        hyper = match (target, anchor) {
                            (Some(t), Some(a)) => Some(format!("{t}#{a}")),
                            (Some(t), None) => Some(t),
                            (None, Some(a)) => Some(format!("#{a}")),
                            (None, None) => None,
                        };
                    }
                    "fldSimple" => {
                        fields.push(Field {
                            instr: String::new(),
                            link: attr(attrs, "instr").and_then(field_link),
                            in_result: true,
                        });
                    }
                    "fldChar" if parent == "r" => match attr(attrs, "fldCharType") {
                        Some("begin") => fields.push(Field::default()),
                        Some("separate") => {
                            if let Some(f) = fields.last_mut() {
                                f.link = field_link(&f.instr);
                                f.in_result = true;
                            }
                        }
                        Some("end") => {
                            fields.pop();
                        }
                        _ => {}
                    },
                    "instrText" if parent == "r" => in_instr = true,
                    "p" => para = Some(ParaBuilder::default()),
                    "r" if para.is_some() => run = Some(RunBuilder::default()),
                    "t" if parent == "r" => in_t = true,
                    "tab" | "ptab" if parent == "r" => {
                        if let Some(r) = run.as_mut() {
                            r.text.push('\t');
                        }
                    }
                    "br" if parent == "r" => {
                        if matches!(attr(attrs, "type"), None | Some("textWrapping")) {
                            if let Some(r) = run.as_mut() {
                                r.text.push('\u{2028}');
                            }
                        }
                    }
                    "cr" if parent == "r" => {
                        if let Some(r) = run.as_mut() {
                            r.text.push('\u{2028}');
                        }
                    }
                    "noBreakHyphen" if parent == "r" => {
                        if let Some(r) = run.as_mut() {
                            r.text.push('-');
                        }
                    }
                    _ if parent == "rPr" && !stack.contains(&"pPr") => {
                        if let Some(r) = run.as_mut() {
                            parse_rpr_child(name, attrs, &mut r.rpr);
                        }
                    }
                    _ if stack.contains(&"pPr") && run.is_none() => {
                        if let Some(p) = para.as_mut() {
                            parse_ppr_child(name, parent, attrs, &mut p.ppr);
                        }
                    }
                    _ => {}
                }
                stack.push(name.as_str());
            }
            Ev::End(name) => {
                if skip > 0 {
                    skip -= 1;
                    continue;
                }
                stack.pop();
                match name.as_str() {
                    "t" => in_t = false,
                    "instrText" => in_instr = false,
                    "hyperlink" => hyper = None,
                    "fldSimple" => {
                        fields.pop();
                    }
                    "r" => {
                        if let (Some(r), Some(p)) = (run.take(), para.as_mut()) {
                            if !r.text.is_empty() {
                                let link = hyper.clone().or_else(|| {
                                    fields.iter().rev().find(|f| f.in_result && f.link.is_some()).and_then(|f| f.link.clone())
                                });
                                p.pending.push((r.text, r.rpr, link));
                            }
                        }
                    }
                    "p" => {
                        run = None;
                        in_t = false;
                        in_instr = false;
                        if let Some(p) = para.take() {
                            let p = finish_paragraph(p, styles, numbering);
                            if tbl_depth == 0 {
                                out.push(Block::Paragraph(p));
                            } else if let Some(c) = cell.as_mut() {
                                c.paragraphs.push(p);
                            }
                        }
                    }
                    "tc" if tbl_depth == 1 && para.is_none() => {
                        if let (Some(c), Some(r)) = (cell.take(), row.as_mut()) {
                            let mut paragraphs = c.paragraphs;
                            if c.vmerge_continue || (paragraphs.len() == 1 && paragraphs[0] == Paragraph::default()) {
                                paragraphs.clear();
                            }
                            r.cells.push(TableCell { paragraphs });
                            for _ in 1..c.span {
                                r.cells.push(TableCell::default());
                            }
                        }
                    }
                    "tr" if tbl_depth == 1 && para.is_none() => {
                        if let Some(r) = row.take() {
                            table.rows.push(r);
                        }
                    }
                    "tbl" => {
                        tbl_depth = tbl_depth.saturating_sub(1);
                        if tbl_depth == 0 {
                            let mut t = std::mem::take(&mut table);
                            row = None;
                            cell = None;
                            if !t.rows.is_empty() {
                                t.make_rectangular();
                                out.push(Block::Table(t));
                            }
                        }
                    }
                    _ => {}
                }
            }
            Ev::Text(t) => {
                if skip == 0 {
                    if in_t {
                        if let Some(r) = run.as_mut() {
                            r.text.push_str(t);
                        }
                    } else if in_instr {
                        if let Some(f) = fields.last_mut() {
                            f.instr.push_str(t);
                        }
                    }
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    const W: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main""#;

    fn docx(body: &str, styles: Option<&str>, numbering: Option<&str>) -> Vec<u8> {
        let mut zw = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let o = SimpleFileOptions::default();
        let mut add = |name: &str, content: String| {
            zw.start_file(name, o).unwrap();
            zw.write_all(content.as_bytes()).unwrap();
        };
        add("[Content_Types].xml", r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"/>"#.into());
        add("_rels/.rels", r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#.into());
        add("word/document.xml", format!(r#"<?xml version="1.0"?><w:document {W}><w:body>{body}</w:body></w:document>"#));
        if let Some(s) = styles {
            add("word/styles.xml", format!(r#"<w:styles {W}>{s}</w:styles>"#));
        }
        if let Some(n) = numbering {
            add("word/numbering.xml", format!(r#"<w:numbering {W}>{n}</w:numbering>"#));
        }
        add("docProps/core.xml", r#"<cp:coreProperties xmlns:cp="a" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>Mon titre</dc:title><dc:creator>Alice</dc:creator></cp:coreProperties>"#.into());
        zw.finish().unwrap().into_inner()
    }

    fn ps(d: &Document) -> Vec<&Paragraph> {
        d.paragraphs().collect()
    }

    fn table(d: &Document) -> &Table {
        d.tables().next().expect("a table")
    }

    fn cell_texts(t: &Table) -> Vec<Vec<String>> {
        t.rows.iter().map(|r| r.cells.iter().map(|c| c.plain_text()).collect()).collect()
    }

    fn docx_with_rels(body: &str, rels: &str) -> Vec<u8> {
        let base = docx(body, None, None);
        let mut zr = zip::ZipArchive::new(Cursor::new(base)).unwrap();
        let mut zw = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for i in 0..zr.len() {
            let f = zr.by_index_raw(i).unwrap();
            zw.raw_copy_file(f).unwrap();
        }
        zw.start_file("word/_rels/document.xml.rels", SimpleFileOptions::default()).unwrap();
        zw.write_all(format!(r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{rels}</Relationships>"#).as_bytes()).unwrap();
        zw.finish().unwrap().into_inner()
    }

    fn parse(body: &str) -> Document {
        read(&docx(body, None, None)).unwrap()
    }

    #[test]
    fn metadata_and_plain() {
        let d = parse(r#"<w:p><w:r><w:t>Bonjour</w:t></w:r></w:p><w:p/>"#);
        assert_eq!(d.metadata.title.as_deref(), Some("Mon titre"));
        assert_eq!(d.metadata.author.as_deref(), Some("Alice"));
        assert_eq!(ps(&d).len(), 2);
        assert_eq!(ps(&d)[0].plain_text(), "Bonjour");
        assert!(ps(&d)[1].runs.is_empty());
    }

    #[test]
    fn toggles_and_merge() {
        let d = parse(
            r#"<w:p><w:r><w:rPr><w:b/><w:i w:val="0"/><w:u w:val="single"/><w:strike w:val="false"/></w:rPr><w:t>a</w:t></w:r>
            <w:r><w:rPr><w:b/><w:u w:val="single"/></w:rPr><w:t>b</w:t></w:r>
            <w:r><w:rPr><w:b w:val="off"/><w:vertAlign w:val="superscript"/></w:rPr><w:t>c</w:t></w:r></w:p>"#,
        );
        let p = &ps(&d)[0];
        assert_eq!(p.runs.len(), 2);
        assert_eq!(p.runs[0].text, "ab");
        assert!(p.runs[0].style.bold && p.runs[0].style.underline && !p.runs[0].style.italic);
        assert!(!p.runs[1].style.bold);
        assert_eq!(p.runs[1].style.vertical, VerticalAlign::Superscript);
    }

    #[test]
    fn tabs_breaks_spaces() {
        let d = parse(
            r#"<w:p><w:r><w:t xml:space="preserve"> a </w:t><w:tab/><w:t>b</w:t><w:br/><w:br w:type="page"/><w:t>c &amp; d</w:t><w:noBreakHyphen/></w:r>
            <w:r><w:delText>gone</w:delText></w:r><w:del><w:r><w:t>del</w:t></w:r></w:del></w:p>"#,
        );
        assert_eq!(ps(&d)[0].plain_text(), " a \tb\u{2028}c & d-");
    }

    #[test]
    fn style_inheritance_and_headings() {
        let styles = r#"<w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii="Serif"/><w:sz w:val="22"/></w:rPr></w:rPrDefault></w:docDefaults>
        <w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style>
        <w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/><w:pPr><w:jc w:val="center"/></w:pPr><w:rPr><w:b/><w:sz w:val="32"/></w:rPr></w:style>
        <w:style w:type="paragraph" w:styleId="MyH"><w:name w:val="My Head"/><w:basedOn w:val="Heading1"/><w:rPr><w:i/></w:rPr></w:style>
        <w:style w:type="paragraph" w:styleId="Out"><w:name w:val="Out"/><w:pPr><w:outlineLvl w:val="1"/></w:pPr></w:style>
        <w:style w:type="paragraph" w:styleId="Q"><w:name w:val="Quote"/></w:style>
        <w:style w:type="character" w:styleId="Strong"><w:name w:val="Strong"/><w:rPr><w:b/><w:color w:val="FF0000"/></w:rPr></w:style>"#;
        let body = r#"<w:p><w:pPr><w:pStyle w:val="MyH"/></w:pPr><w:r><w:t>H</w:t></w:r></w:p>
        <w:p><w:pPr><w:pStyle w:val="Out"/></w:pPr><w:r><w:t>O</w:t></w:r></w:p>
        <w:p><w:pPr><w:pStyle w:val="Q"/></w:pPr><w:r><w:t>Q</w:t></w:r></w:p>
        <w:p><w:pPr><w:pStyle w:val="Heading7"/></w:pPr><w:r><w:t>X</w:t></w:r></w:p>
        <w:p><w:r><w:rPr><w:rStyle w:val="Strong"/><w:sz w:val="40"/><w:color w:val="auto"/></w:rPr><w:t>S</w:t></w:r></w:p>"#;
        let d = read(&docx(body, Some(styles), None)).unwrap();
        let h = &ps(&d)[0];
        assert_eq!(h.style, ParagraphStyle::Heading1);
        assert_eq!(h.alignment, Alignment::Left);
        // Everything comes from the heading style: no direct formatting.
        assert_eq!(h.runs[0].style, CharStyle::default());
        assert_eq!(ps(&d)[1].style, ParagraphStyle::Heading2);
        assert_eq!(ps(&d)[2].style, ParagraphStyle::Quote);
        assert_eq!(ps(&d)[3].style, ParagraphStyle::Heading3);
        let s = &ps(&d)[4].runs[0].style;
        assert!(s.bold);
        assert_eq!(s.color, Some(Rgb(255, 0, 0)));
        assert_eq!(s.font_size, Some(20.0));
    }

    #[test]
    fn lists() {
        let numbering = r#"<w:abstractNum w:abstractNumId="0"><w:lvl w:ilvl="0"><w:numFmt w:val="bullet"/></w:lvl></w:abstractNum>
        <w:abstractNum w:abstractNumId="1"><w:lvl w:ilvl="0"><w:numFmt w:val="decimal"/></w:lvl><w:lvl w:ilvl="1"><w:numFmt w:val="bullet"/></w:lvl></w:abstractNum>
        <w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num><w:num w:numId="2"><w:abstractNumId w:val="1"/></w:num>"#;
        let styles = r#"<w:style w:type="paragraph" w:styleId="LB"><w:name w:val="List Bullet"/><w:pPr><w:numPr><w:numId w:val="1"/></w:numPr></w:pPr></w:style>"#;
        let body = r#"<w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr></w:pPr><w:r><w:t>a</w:t></w:r></w:p>
        <w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="2"/></w:numPr><w:rPr><w:b/></w:rPr></w:pPr><w:r><w:t>b</w:t></w:r></w:p>
        <w:p><w:pPr><w:numPr><w:ilvl w:val="1"/><w:numId w:val="2"/></w:numPr></w:pPr><w:r><w:t>c</w:t></w:r></w:p>
        <w:p><w:pPr><w:pStyle w:val="LB"/></w:pPr><w:r><w:t>d</w:t></w:r></w:p>
        <w:p><w:pPr><w:numPr><w:numId w:val="0"/></w:numPr></w:pPr><w:r><w:t>e</w:t></w:r></w:p>"#;
        let d = read(&docx(body, Some(styles), Some(numbering))).unwrap();
        let kinds: Vec<_> = d.paragraphs().map(|p| p.list).collect();
        assert_eq!(
            kinds,
            vec![Some(ListKind::Bullet), Some(ListKind::Numbered), Some(ListKind::Bullet), Some(ListKind::Bullet), None]
        );
        assert!(!ps(&d)[1].runs[0].style.bold);
        // No numbering part: bullets.
        let d = parse(r#"<w:p><w:pPr><w:numPr><w:numId w:val="3"/></w:numPr></w:pPr><w:r><w:t>x</w:t></w:r></w:p>"#);
        assert_eq!(ps(&d)[0].list, Some(ListKind::Bullet));
    }

    #[test]
    fn alignment_highlight_size() {
        let body = r#"<w:p><w:pPr><w:jc w:val="both"/></w:pPr><w:r><w:rPr><w:highlight w:val="yellow"/><w:sz w:val="25"/></w:rPr><w:t>J</w:t></w:r></w:p>
        <w:tbl><w:tr><w:tc><w:p><w:pPr><w:jc w:val="end"/></w:pPr><w:hyperlink><w:r><w:rPr><w:shd w:fill="00FF00" w:val="clear"/></w:rPr><w:t>T</w:t></w:r></w:hyperlink></w:p></w:tc></w:tr></w:tbl>
        <w:p><w:r><w:drawing><w:p><w:r><w:t>no</w:t></w:r></w:p></w:drawing></w:r></w:p><w:sectPr/>"#;
        let d = parse(body);
        assert_eq!(ps(&d).len(), 2);
        assert_eq!(ps(&d)[0].alignment, Alignment::Justify);
        let s = &ps(&d)[0].runs[0].style;
        assert_eq!(s.highlight, Some(Rgb(255, 255, 0)));
        assert_eq!(s.font_size, Some(12.5));
        let c = &table(&d).rows[0].cells[0].paragraphs[0];
        assert_eq!(c.alignment, Alignment::Right);
        assert_eq!(c.runs[0].style.highlight, Some(Rgb(0, 255, 0)));
        assert_eq!(c.runs[0].style.link, None);
        assert!(ps(&d)[1].runs.is_empty());
    }

    fn cell(t: &str) -> String {
        format!("<w:tc><w:p><w:r><w:t>{t}</w:t></w:r></w:p></w:tc>")
    }

    #[test]
    fn table_basic_and_rectangular() {
        let body = format!(
            "<w:p><w:r><w:t>avant</w:t></w:r></w:p><w:tbl><w:tblPr><w:tblW w:w=\"5000\"/></w:tblPr><w:tblGrid><w:gridCol w:w=\"1\"/></w:tblGrid>\
             <w:tr><w:trPr/>{}{}</w:tr><w:tr>{}</w:tr></w:tbl><w:p><w:r><w:t>apres</w:t></w:r></w:p>",
            cell("a"),
            cell("b"),
            cell("c")
        );
        let d = parse(&body);
        assert_eq!(d.blocks.len(), 3);
        assert_eq!(cell_texts(table(&d)), vec![vec!["a", "b"], vec!["c", ""]]);
        assert!(table(&d).rows[1].cells[1].paragraphs.is_empty());
    }

    #[test]
    fn table_grid_span_and_vmerge() {
        let body = format!(
            "<w:tbl><w:tr><w:tc><w:tcPr><w:gridSpan w:val=\"3\"/></w:tcPr><w:p><w:r><w:t>wide</w:t></w:r></w:p></w:tc></w:tr>\
             <w:tr><w:tc><w:tcPr><w:vMerge w:val=\"restart\"/></w:tcPr><w:p><w:r><w:t>top</w:t></w:r></w:p></w:tc>{}{}</w:tr>\
             <w:tr><w:tc><w:tcPr><w:vMerge/></w:tcPr><w:p><w:r><w:t>hidden</w:t></w:r></w:p></w:tc>{}{}</w:tr></w:tbl>",
            cell("x"),
            cell("y"),
            cell("z"),
            cell("w")
        );
        let d = parse(&body);
        assert_eq!(
            cell_texts(table(&d)),
            vec![vec!["wide", "", ""], vec!["top", "x", "y"], vec!["", "z", "w"]]
        );
    }

    #[test]
    fn table_nested_is_flattened() {
        let body = "<w:tbl><w:tr><w:tc><w:p><w:r><w:t>one</w:t></w:r></w:p>\
            <w:tbl><w:tr><w:tc><w:p><w:r><w:t>in1</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>in2</w:t></w:r></w:p></w:tc></w:tr></w:tbl>\
            <w:p><w:r><w:t>two</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>next</w:t></w:r></w:p></w:tc></w:tr></w:tbl>";
        let d = parse(body);
        let t = table(&d);
        assert_eq!(t.rows.len(), 1);
        assert_eq!(t.rows[0].cells.len(), 2);
        let texts: Vec<String> = t.rows[0].cells[0].paragraphs.iter().map(|p| p.plain_text()).collect();
        assert_eq!(texts, vec!["one", "in1", "in2", "two"]);
        assert_eq!(t.rows[0].cells[1].plain_text(), "next");
        assert_eq!(d.blocks.len(), 1);
    }

    #[test]
    fn hyperlink_via_relationship_and_anchor() {
        let body = r#"<w:p><w:r><w:t>a </w:t></w:r><w:hyperlink r:id="rId9" xmlns:r="urn:r"><w:r><w:rPr><w:rStyle w:val="Hyperlink"/><w:b/></w:rPr><w:t>lien</w:t></w:r></w:hyperlink>
            <w:hyperlink w:anchor="sec1"><w:r><w:t>interne</w:t></w:r></w:hyperlink><w:hyperlink r:id="nope"><w:r><w:t>x</w:t></w:r></w:hyperlink></w:p>"#;
        let rels = r#"<Relationship Id="rId9" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="https://exemple.org/?a=1&amp;b=2" TargetMode="External"/>"#;
        let styles = r#"<w:style w:type="character" w:styleId="Hyperlink"><w:name w:val="Hyperlink"/><w:rPr><w:color w:val="0563C1"/><w:u w:val="single"/></w:rPr></w:style>"#;
        // Build with styles too.
        let base = docx_with_rels(&body.replace("<w:p>", "<w:p xmlns:r=\"urn:r\">"), rels);
        let mut zr = zip::ZipArchive::new(Cursor::new(base)).unwrap();
        let mut zw = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for i in 0..zr.len() {
            let f = zr.by_index_raw(i).unwrap();
            zw.raw_copy_file(f).unwrap();
        }
        zw.start_file("word/styles.xml", SimpleFileOptions::default()).unwrap();
        zw.write_all(format!(r#"<w:styles {W}>{styles}</w:styles>"#).as_bytes()).unwrap();
        let d = read(&zw.finish().unwrap().into_inner()).unwrap();
        let runs = &ps(&d)[0].runs;
        assert_eq!(runs.len(), 4, "{runs:?}");
        assert_eq!(runs[0].style.link, None);
        assert_eq!(runs[1].text, "lien");
        assert_eq!(runs[1].style.link.as_deref(), Some("https://exemple.org/?a=1&b=2"));
        assert!(runs[1].style.bold);
        assert!(!runs[1].style.underline);
        assert_eq!(runs[1].style.color, None);
        assert_eq!(runs[2].style.link.as_deref(), Some("#sec1"));
        assert_eq!(runs[3].style.link, None);
    }

    #[test]
    fn hyperlink_field_codes() {
        let body = r#"<w:p>
            <w:fldSimple w:instr=" HYPERLINK &quot;https://a.org/&quot; "><w:r><w:t>simple</w:t></w:r></w:fldSimple>
            <w:r><w:t> mid </w:t></w:r>
            <w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> HYPERLINK "https://b.org/" \o "tip" </w:instrText></w:r>
            <w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>complexe</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r>
            <w:r><w:t> fin </w:t></w:r>
            <w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText> PAGE </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>3</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r>
            <w:fldSimple w:instr="HYPERLINK \l &quot;ancre&quot;"><w:r><w:t>loc</w:t></w:r></w:fldSimple></w:p>"#;
        let d = parse(body);
        let runs = &ps(&d)[0].runs;
        let got: Vec<(&str, Option<&str>)> = runs.iter().map(|r| (r.text.as_str(), r.style.link.as_deref())).collect();
        assert_eq!(
            got,
            vec![
                ("simple", Some("https://a.org/")),
                (" mid ", None),
                ("complexe", Some("https://b.org/")),
                (" fin 3", None),
                ("loc", Some("#ancre")),
            ]
        );
    }

    #[test]
    fn invalid_input_errors() {
        assert!(read(b"not a zip").is_err());
    }
}
