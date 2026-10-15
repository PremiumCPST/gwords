//! OpenDocument Text 1.3 writer.

use std::collections::BTreeSet;
use std::collections::HashMap;
use std::io::{Cursor, Write};

use zip::CompressionMethod;
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

use crate::document::{
    Alignment, Block, CharStyle, Document, ListKind, Paragraph, ParagraphStyle, Table, VerticalAlign,
};
use crate::formats::FormatError;

const MIMETYPE: &str = "application/vnd.oasis.opendocument.text";

const NS: &str = "xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" \
xmlns:style=\"urn:oasis:names:tc:opendocument:xmlns:style:1.0\" \
xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\" \
xmlns:table=\"urn:oasis:names:tc:opendocument:xmlns:table:1.0\" \
xmlns:draw=\"urn:oasis:names:tc:opendocument:xmlns:drawing:1.0\" \
xmlns:fo=\"urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0\" \
xmlns:xlink=\"http://www.w3.org/1999/xlink\" \
xmlns:dc=\"http://purl.org/dc/elements/1.1/\" \
xmlns:meta=\"urn:oasis:names:tc:opendocument:xmlns:meta:1.0\" \
xmlns:svg=\"urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0\" \
xmlns:config=\"urn:oasis:names:tc:opendocument:xmlns:config:1.0\" \
xmlns:loext=\"urn:org:documentfoundation:names:experimental:office:xmlns:loext:1.0\"";

const XML_DECL: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n";

fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&apos;"),
            c if (c as u32) < 0x20 && c != '\t' && c != '\n' && c != '\r' => {}
            c => o.push(c),
        }
    }
    o
}

fn fmt_num(v: f64) -> String {
    let s = format!("{:.2}", v);
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// Deduplicating registry of automatic styles.
#[derive(Default)]
struct StyleSet {
    /// (xml body, name) in creation order.
    text: Vec<(String, String)>,
    text_idx: HashMap<String, String>,
    para: Vec<(String, String, Alignment)>, // (name, parent, align)
    para_idx: HashMap<(String, Alignment), String>,
    fonts: BTreeSet<String>,
    /// Column count of each table written so far (table N has style `TableauN`).
    tables: Vec<usize>,
}

/// Text width of the page (A4 minus 2 cm margins), in cm.
const TABLE_WIDTH_CM: f64 = 17.0;
const CELL_STYLE: &str = "TableauCell";

impl StyleSet {
    fn text_style(&mut self, cs: &CharStyle) -> Option<String> {
        let mut p = String::new();
        if cs.bold {
            p.push_str(" fo:font-weight=\"bold\" style:font-weight-asian=\"bold\" style:font-weight-complex=\"bold\"");
        }
        if cs.italic {
            p.push_str(" fo:font-style=\"italic\" style:font-style-asian=\"italic\" style:font-style-complex=\"italic\"");
        }
        if cs.underline {
            p.push_str(" style:text-underline-style=\"solid\" style:text-underline-width=\"auto\" style:text-underline-color=\"font-color\"");
        }
        if cs.strikethrough {
            p.push_str(" style:text-line-through-style=\"solid\"");
        }
        match cs.vertical {
            VerticalAlign::Superscript => p.push_str(" style:text-position=\"super 58%\""),
            VerticalAlign::Subscript => p.push_str(" style:text-position=\"sub 58%\""),
            VerticalAlign::Baseline => {}
        }
        if let Some(sz) = cs.font_size {
            let n = fmt_num(sz);
            p.push_str(&format!(
                " fo:font-size=\"{n}pt\" style:font-size-asian=\"{n}pt\" style:font-size-complex=\"{n}pt\""
            ));
        }
        if let Some(f) = cs.font_family.as_deref().filter(|f| !f.is_empty()) {
            self.fonts.insert(f.to_string());
            p.push_str(&format!(" style:font-name=\"{}\"", esc(f)));
        }
        if let Some(c) = cs.color {
            p.push_str(&format!(" fo:color=\"#{}\"", c.to_hex()));
        }
        if let Some(c) = cs.highlight {
            p.push_str(&format!(" fo:background-color=\"#{}\"", c.to_hex()));
        }
        if p.is_empty() {
            return None;
        }
        if let Some(n) = self.text_idx.get(&p) {
            return Some(n.clone());
        }
        let name = format!("T{}", self.text.len() + 1);
        self.text_idx.insert(p.clone(), name.clone());
        self.text.push((p, name.clone()));
        Some(name)
    }

    fn para_style(&mut self, parent: &str, align: Alignment) -> String {
        if align == Alignment::Left {
            return parent.to_string();
        }
        let key = (parent.to_string(), align);
        if let Some(n) = self.para_idx.get(&key) {
            return n.clone();
        }
        let name = format!("P{}", self.para.len() + 1);
        self.para_idx.insert(key, name.clone());
        self.para.push((name.clone(), parent.to_string(), align));
        name
    }

    fn to_xml(&self) -> String {
        let mut o = String::new();
        for (name, parent, align) in &self.para {
            let a = match align {
                Alignment::Left => "start",
                Alignment::Center => "center",
                Alignment::Right => "end",
                Alignment::Justify => "justify",
            };
            o.push_str(&format!(
                "<style:style style:name=\"{name}\" style:family=\"paragraph\" style:parent-style-name=\"{parent}\">\
<style:paragraph-properties fo:text-align=\"{a}\"/></style:style>"
            ));
        }
        for (i, cols) in self.tables.iter().enumerate() {
            let n = i + 1;
            let cols = (*cols).max(1);
            let width = TABLE_WIDTH_CM / cols as f64;
            let rel = 65535 / cols;
            o.push_str(&format!(
                "<style:style style:name=\"Tableau{n}\" style:family=\"table\">\
<style:table-properties style:width=\"{TABLE_WIDTH_CM:.3}cm\" style:rel-width=\"100%\" table:align=\"margins\"/></style:style>\
<style:style style:name=\"Tableau{n}.A\" style:family=\"table-column\">\
<style:table-column-properties style:column-width=\"{width:.3}cm\" style:rel-column-width=\"{rel}*\"/></style:style>"
            ));
        }
        if !self.tables.is_empty() {
            o.push_str(&format!(
                "<style:style style:name=\"{CELL_STYLE}\" style:family=\"table-cell\">\
<style:table-cell-properties fo:padding=\"0.1cm\" fo:border=\"0.5pt solid #000000\"/></style:style>"
            ));
        }
        for (props, name) in &self.text {
            o.push_str(&format!(
                "<style:style style:name=\"{name}\" style:family=\"text\"><style:text-properties{props}/></style:style>"
            ));
        }
        o
    }
}

fn style_name(s: ParagraphStyle) -> &'static str {
    match s {
        ParagraphStyle::Normal => "Standard",
        ParagraphStyle::Title => "Title",
        ParagraphStyle::Heading1 => "Heading_20_1",
        ParagraphStyle::Heading2 => "Heading_20_2",
        ParagraphStyle::Heading3 => "Heading_20_3",
        ParagraphStyle::Quote => "Quotations",
        ParagraphStyle::Code => "Preformatted_20_Text",
    }
}

fn outline_level(s: ParagraphStyle) -> Option<u8> {
    match s {
        ParagraphStyle::Heading1 => Some(1),
        ParagraphStyle::Heading2 => Some(2),
        ParagraphStyle::Heading3 => Some(3),
        _ => None,
    }
}

fn write_paragraph(out: &mut String, p: &Paragraph, styles: &mut StyleSet) {
    let pname = styles.para_style(style_name(p.style), p.alignment);
    let (open, close) = match outline_level(p.style) {
        Some(l) => (
            format!("<text:h text:style-name=\"{pname}\" text:outline-level=\"{l}\">"),
            "</text:h>",
        ),
        None => (format!("<text:p text:style-name=\"{pname}\">"), "</text:p>"),
    };
    out.push_str(&open);
    // prev_space: previous emitted char was a (literal or encoded) space.
    let mut prev_space = true; // paragraph start counts as whitespace
    let mut open_link: Option<&str> = None;
    for run in &p.runs {
        if run.text.is_empty() {
            continue;
        }
        let link = run.style.link.as_deref().filter(|l| !l.is_empty());
        if link != open_link {
            if open_link.is_some() {
                out.push_str("</text:a>");
            }
            if let Some(url) = link {
                out.push_str(&format!("<text:a xlink:type=\"simple\" xlink:href=\"{}\">", esc(url)));
            }
            open_link = link;
        }
        let sname = styles.text_style(&run.style);
        if let Some(n) = &sname {
            out.push_str(&format!("<text:span text:style-name=\"{n}\">"));
        }
        let mut buf = String::new();
        let mut extra = 0usize;
        let flush_extra = |out: &mut String, extra: &mut usize| {
            if *extra == 1 {
                out.push_str("<text:s/>");
            } else if *extra > 1 {
                out.push_str(&format!("<text:s text:c=\"{}\"/>", *extra));
            }
            *extra = 0;
        };
        for c in run.text.chars() {
            if c == ' ' {
                if prev_space {
                    if !buf.is_empty() {
                        out.push_str(&esc(&buf));
                        buf.clear();
                    }
                    extra += 1;
                } else {
                    buf.push(' ');
                }
                prev_space = true;
                continue;
            }
            if extra > 0 {
                if !buf.is_empty() {
                    out.push_str(&esc(&buf));
                    buf.clear();
                }
                flush_extra(out, &mut extra);
            }
            prev_space = false;
            match c {
                '\t' | '\u{2028}' => {
                    if !buf.is_empty() {
                        out.push_str(&esc(&buf));
                        buf.clear();
                    }
                    out.push_str(if c == '\t' { "<text:tab/>" } else { "<text:line-break/>" });
                }
                '\n' | '\r' => {
                    if !buf.is_empty() {
                        out.push_str(&esc(&buf));
                        buf.clear();
                    }
                    if c == '\n' {
                        out.push_str("<text:line-break/>");
                    }
                }
                c => buf.push(c),
            }
        }
        if !buf.is_empty() {
            out.push_str(&esc(&buf));
        }
        flush_extra(out, &mut extra);
        if sname.is_some() {
            out.push_str("</text:span>");
        }
    }
    if open_link.is_some() {
        out.push_str("</text:a>");
    }
    out.push_str(close);
}

fn font_face(name: &str, generic: Option<&str>) -> String {
    let n = esc(name);
    let g = generic.map(|g| format!(" style:font-family-generic=\"{g}\"")).unwrap_or_default();
    format!("<style:font-face style:name=\"{n}\" svg:font-family=\"{n}\"{g}/>")
}

fn font_decls(extra: &BTreeSet<String>) -> String {
    let mut all: BTreeSet<&str> = ["Cantarell", "Liberation Mono"].into_iter().collect();
    for f in extra {
        all.insert(f);
    }
    let mut o = String::from("<office:font-face-decls>");
    for f in all {
        let g = match f {
            "Liberation Mono" => Some("modern"),
            _ => None,
        };
        o.push_str(&font_face(f, g));
    }
    o.push_str("</office:font-face-decls>");
    o
}

fn list_style(name: &str, kind: ListKind) -> String {
    let mut o = format!("<text:list-style style:name=\"{name}\">");
    for lvl in 1..=10 {
        let indent = 0.635 * lvl as f64;
        let props = format!(
            "<style:list-level-properties text:list-level-position-and-space-mode=\"label-alignment\">\
<style:list-level-label-alignment text:label-followed-by=\"listtab\" text:list-tab-stop-position=\"{indent:.3}cm\" fo:text-indent=\"-0.635cm\" fo:margin-left=\"{indent:.3}cm\"/>\
</style:list-level-properties>"
        );
        match kind {
            ListKind::Bullet => o.push_str(&format!(
                "<text:list-level-style-bullet text:level=\"{lvl}\" text:bullet-char=\"•\">{props}</text:list-level-style-bullet>"
            )),
            ListKind::Numbered => o.push_str(&format!(
                "<text:list-level-style-number text:level=\"{lvl}\" style:num-suffix=\".\" style:num-format=\"1\">{props}</text:list-level-style-number>"
            )),
        }
    }
    o.push_str("</text:list-style>");
    o
}

fn para_style_def(name: &str, display: &str, parent: Option<&str>, next: Option<&str>, class: &str, extra: &str) -> String {
    let mut o = format!("<style:style style:name=\"{name}\"");
    if !display.is_empty() {
        o.push_str(&format!(" style:display-name=\"{display}\""));
    }
    o.push_str(" style:family=\"paragraph\"");
    if let Some(p) = parent {
        o.push_str(&format!(" style:parent-style-name=\"{p}\""));
    }
    if let Some(n) = next {
        o.push_str(&format!(" style:next-style-name=\"{n}\""));
    }
    o.push_str(&format!(" style:class=\"{class}\">{extra}</style:style>"));
    o
}

fn styles_xml() -> String {
    let mut o = String::new();
    o.push_str(XML_DECL);
    o.push_str(&format!("<office:document-styles {NS} office:version=\"1.3\">"));
    o.push_str(&font_decls(&BTreeSet::new()));
    o.push_str("<office:styles>");
    o.push_str(
        "<style:default-style style:family=\"paragraph\">\
<style:paragraph-properties fo:orphans=\"2\" fo:widows=\"2\" style:writing-mode=\"page\"/>\
<style:text-properties style:font-name=\"Cantarell\" fo:font-size=\"12pt\" fo:language=\"fr\" fo:country=\"FR\"/>\
</style:default-style>",
    );
    o.push_str(&para_style_def("Standard", "", None, None, "text", ""));
    o.push_str(&para_style_def(
        "Heading",
        "Heading",
        Some("Standard"),
        Some("Text_20_body"),
        "text",
        "<style:paragraph-properties fo:margin-top=\"0.423cm\" fo:margin-bottom=\"0.212cm\" fo:keep-with-next=\"always\"/>\
<style:text-properties fo:font-weight=\"bold\" style:font-weight-asian=\"bold\" style:font-weight-complex=\"bold\"/>",
    ));
    o.push_str(&para_style_def(
        "Text_20_body",
        "Text body",
        Some("Standard"),
        None,
        "text",
        "<style:paragraph-properties fo:margin-top=\"0cm\" fo:margin-bottom=\"0.247cm\"/>",
    ));
    o.push_str(&para_style_def(
        "Title",
        "Title",
        Some("Heading"),
        Some("Text_20_body"),
        "chapter",
        "<style:paragraph-properties fo:text-align=\"center\"/><style:text-properties fo:font-size=\"26pt\" style:font-size-asian=\"26pt\" style:font-size-complex=\"26pt\"/>",
    ));
    for (n, sz) in [(1, 20), (2, 16), (3, 14)] {
        o.push_str(&format!(
            "<style:style style:name=\"Heading_20_{n}\" style:display-name=\"Heading {n}\" style:family=\"paragraph\" \
style:parent-style-name=\"Heading\" style:next-style-name=\"Text_20_body\" style:default-outline-level=\"{n}\" style:class=\"text\">\
<style:text-properties fo:font-size=\"{sz}pt\" fo:font-weight=\"bold\" style:font-size-asian=\"{sz}pt\" style:font-size-complex=\"{sz}pt\" \
style:font-weight-asian=\"bold\" style:font-weight-complex=\"bold\"/></style:style>"
        ));
    }
    o.push_str(&para_style_def(
        "Quotations",
        "Quotations",
        Some("Standard"),
        None,
        "html",
        "<style:paragraph-properties fo:margin-left=\"1cm\" fo:margin-right=\"1cm\" fo:margin-top=\"0cm\" fo:margin-bottom=\"0.25cm\" fo:text-indent=\"0cm\"/>\
<style:text-properties fo:font-style=\"italic\" style:font-style-asian=\"italic\" style:font-style-complex=\"italic\"/>",
    ));
    o.push_str(&para_style_def(
        "Preformatted_20_Text",
        "Preformatted Text",
        Some("Standard"),
        None,
        "html",
        "<style:paragraph-properties fo:margin-top=\"0cm\" fo:margin-bottom=\"0cm\"/>\
<style:text-properties style:font-name=\"Liberation Mono\" fo:font-size=\"10pt\" style:font-size-asian=\"10pt\" style:font-size-complex=\"10pt\"/>",
    ));
    o.push_str(&list_style("Bullet_20_List", ListKind::Bullet).replace("style:name=\"Bullet_20_List\"", "style:name=\"Bullet_20_List\" style:display-name=\"Bullet List\""));
    o.push_str(&list_style("Numbering_20_List", ListKind::Numbered).replace("style:name=\"Numbering_20_List\"", "style:name=\"Numbering_20_List\" style:display-name=\"Numbering List\""));
    o.push_str("</office:styles>");
    o.push_str(
        "<office:automatic-styles>\
<style:page-layout style:name=\"Mpm1\">\
<style:page-layout-properties fo:page-width=\"21.001cm\" fo:page-height=\"29.7cm\" style:print-orientation=\"portrait\" \
fo:margin-top=\"2cm\" fo:margin-bottom=\"2cm\" fo:margin-left=\"2cm\" fo:margin-right=\"2cm\" style:writing-mode=\"lr-tb\"/>\
</style:page-layout></office:automatic-styles>\
<office:master-styles><style:master-page style:name=\"Standard\" style:page-layout-name=\"Mpm1\"/></office:master-styles>",
    );
    o.push_str("</office:document-styles>");
    o
}

/// Writes consecutive paragraphs, grouping list items into `text:list`.
fn write_paragraphs(out: &mut String, paras: &[&Paragraph], styles: &mut StyleSet) {
    let mut i = 0;
    while i < paras.len() {
        match paras[i].list {
            None => {
                write_paragraph(out, paras[i], styles);
                i += 1;
            }
            Some(kind) => {
                let lname = match kind {
                    ListKind::Bullet => "Bullet_20_List",
                    ListKind::Numbered => "Numbering_20_List",
                };
                out.push_str(&format!("<text:list text:style-name=\"{lname}\">"));
                while i < paras.len() && paras[i].list == Some(kind) {
                    out.push_str("<text:list-item>");
                    write_paragraph(out, paras[i], styles);
                    out.push_str("</text:list-item>");
                    i += 1;
                }
                out.push_str("</text:list>");
            }
        }
    }
}

fn write_table(out: &mut String, table: &Table, styles: &mut StyleSet) {
    let mut table = table.clone();
    table.make_rectangular();
    let cols = table.column_count();
    styles.tables.push(cols);
    let n = styles.tables.len();
    out.push_str(&format!(
        "<table:table table:name=\"Tableau{n}\" table:style-name=\"Tableau{n}\">\
<table:table-column table:style-name=\"Tableau{n}.A\" table:number-columns-repeated=\"{cols}\"/>"
    ));
    for row in &table.rows {
        out.push_str("<table:table-row>");
        for cell in &row.cells {
            out.push_str(&format!(
                "<table:table-cell table:style-name=\"{CELL_STYLE}\" office:value-type=\"string\">"
            ));
            if cell.paragraphs.is_empty() {
                out.push_str("<text:p text:style-name=\"Standard\"/>");
            } else {
                let paras: Vec<&Paragraph> = cell.paragraphs.iter().collect();
                write_paragraphs(out, &paras, styles);
            }
            out.push_str("</table:table-cell>");
        }
        out.push_str("</table:table-row>");
    }
    out.push_str("</table:table>");
}

fn content_xml(doc: &Document) -> String {
    let mut styles = StyleSet::default();
    let mut body = String::new();
    let mut pending: Vec<&Paragraph> = Vec::new();
    for block in &doc.blocks {
        match block {
            Block::Paragraph(p) => pending.push(p),
            Block::Table(t) => {
                write_paragraphs(&mut body, &pending, &mut styles);
                pending.clear();
                write_table(&mut body, t, &mut styles);
            }
        }
    }
    write_paragraphs(&mut body, &pending, &mut styles);
    if doc.blocks.is_empty() {
        body.push_str("<text:p text:style-name=\"Standard\"/>");
    }
    let mut o = String::new();
    o.push_str(XML_DECL);
    o.push_str(&format!("<office:document-content {NS} office:version=\"1.3\">"));
    o.push_str(&font_decls(&styles.fonts));
    o.push_str("<office:automatic-styles>");
    o.push_str(&styles.to_xml());
    o.push_str("</office:automatic-styles><office:body><office:text>");
    o.push_str(&body);
    o.push_str("</office:text></office:body></office:document-content>");
    o
}

fn meta_xml(doc: &Document) -> String {
    let mut o = String::new();
    o.push_str(XML_DECL);
    o.push_str(&format!("<office:document-meta {NS} office:version=\"1.3\"><office:meta>"));
    o.push_str("<meta:generator>Gwords</meta:generator>");
    if let Some(t) = doc.metadata.title.as_deref().filter(|t| !t.is_empty()) {
        o.push_str(&format!("<dc:title>{}</dc:title>", esc(t)));
    }
    if let Some(a) = doc.metadata.author.as_deref().filter(|a| !a.is_empty()) {
        o.push_str(&format!("<meta:initial-creator>{0}</meta:initial-creator><dc:creator>{0}</dc:creator>", esc(a)));
    }
    o.push_str("</office:meta></office:document-meta>");
    o
}

fn settings_xml() -> String {
    format!(
        "{XML_DECL}<office:document-settings {NS} office:version=\"1.3\"><office:settings>\
<config:config-item-set config:name=\"ooo:view-settings\"/></office:settings></office:document-settings>"
    )
}

fn manifest_xml() -> String {
    let mut o = String::new();
    o.push_str(XML_DECL);
    o.push_str(
        "<manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\" manifest:version=\"1.3\">\
<manifest:file-entry manifest:full-path=\"/\" manifest:version=\"1.3\" manifest:media-type=\"application/vnd.oasis.opendocument.text\"/>",
    );
    for f in ["content.xml", "styles.xml", "meta.xml", "settings.xml"] {
        o.push_str(&format!(
            "<manifest:file-entry manifest:full-path=\"{f}\" manifest:media-type=\"text/xml\"/>"
        ));
    }
    o.push_str("</manifest:manifest>");
    o
}

pub fn write(doc: &Document) -> Result<Vec<u8>, FormatError> {
    let mut zw = ZipWriter::new(Cursor::new(Vec::new()));
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    zw.start_file("mimetype", stored)?;
    zw.write_all(MIMETYPE.as_bytes())?;
    let files: [(&str, String); 5] = [
        ("META-INF/manifest.xml", manifest_xml()),
        ("content.xml", content_xml(doc)),
        ("styles.xml", styles_xml()),
        ("meta.xml", meta_xml(doc)),
        ("settings.xml", settings_xml()),
    ];
    for (name, data) in files {
        zw.start_file(name, deflated)?;
        zw.write_all(data.as_bytes())?;
    }
    Ok(zw.finish()?.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Metadata, Rgb, Run, TableCell, TableRow};
    use std::io::Read;

    fn read_entry(bytes: &[u8], name: &str) -> String {
        let mut z = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        let mut s = String::new();
        z.by_name(name).unwrap().read_to_string(&mut s).unwrap();
        s
    }

    fn well_formed(xml: &str) {
        let mut r = quick_xml::Reader::from_str(xml);
        loop {
            match r.read_event().expect("well-formed xml") {
                quick_xml::events::Event::Eof => break,
                _ => {}
            }
        }
    }

    fn run(t: &str, style: CharStyle) -> Run {
        Run { text: t.into(), style }
    }

    fn sample() -> Document {
        let bold = CharStyle { bold: true, color: Some(Rgb(255, 0, 0)), font_family: Some("Foo & Bar".into()), ..Default::default() };
        Document {
            metadata: Metadata { title: Some("T <1>".into()), author: Some("Moi".into()) },
            blocks: vec![
                Block::Paragraph(Paragraph { style: ParagraphStyle::Title, runs: vec![Run::plain("Titre")], ..Default::default() }),
                Block::Paragraph(Paragraph { style: ParagraphStyle::Heading2, alignment: Alignment::Center, runs: vec![Run::plain("H2")], ..Default::default() }),
                Block::Paragraph(Paragraph {
                    alignment: Alignment::Center,
                    runs: vec![run("a", bold.clone()), run("b", bold.clone()), Run::plain("x &  <y>   z\tw\u{2028}v")],
                    ..Default::default()
                }),
                Block::Paragraph(Paragraph { runs: vec![Run::plain(" lead")], ..Default::default() }),
                Block::Paragraph(Paragraph { list: Some(ListKind::Bullet), runs: vec![Run::plain("one")], ..Default::default() }),
                Block::Paragraph(Paragraph { list: Some(ListKind::Bullet), runs: vec![Run::plain("two")], ..Default::default() }),
                Block::Paragraph(Paragraph { list: Some(ListKind::Numbered), runs: vec![Run::plain("n")], ..Default::default() }),
            ],
        }
    }

    #[test]
    fn zip_structure() {
        let bytes = write(&sample()).unwrap();
        let mut z = zip::ZipArchive::new(Cursor::new(&bytes[..])).unwrap();
        {
            let f = z.by_index(0).unwrap();
            assert_eq!(f.name().unwrap(), "mimetype");
            assert_eq!(f.compression(), CompressionMethod::Stored);
            assert!(f.extra_data().map(|e| e.is_empty()).unwrap_or(true));
        }
        for n in ["META-INF/manifest.xml", "content.xml", "styles.xml", "meta.xml", "settings.xml"] {
            well_formed(&read_entry(&bytes, n));
        }
        assert_eq!(read_entry(&bytes, "mimetype"), MIMETYPE);
        let meta = read_entry(&bytes, "meta.xml");
        assert!(meta.contains("<dc:title>T &lt;1&gt;</dc:title>"));
        assert!(meta.contains("Gwords"));
    }

    #[test]
    fn content_encoding_and_dedup() {
        let bytes = write(&sample()).unwrap();
        let c = read_entry(&bytes, "content.xml");
        assert!(c.contains("<text:h text:style-name=\"P1\" text:outline-level=\"2\">"));
        assert!(c.contains("<text:p text:style-name=\"Title\">"));
        // Centered paragraphs share P2 (H2 differs by parent => P1, normal => P2).
        assert_eq!(c.matches("style:family=\"paragraph\"").count(), 2);
        // Identical char styles deduplicated.
        assert_eq!(c.matches("style:family=\"text\"").count(), 1);
        assert!(c.contains("x &amp; <text:s/>&lt;y&gt; <text:s text:c=\"2\"/>z<text:tab/>w<text:line-break/>v"));
        assert!(c.contains("<text:s/>lead"));
        assert_eq!(c.matches("<text:list ").count(), 2);
        assert_eq!(c.matches("<text:list-item>").count(), 3);
        assert!(c.contains("style:font-name=\"Foo &amp; Bar\""));
        assert!(c.contains("fo:color=\"#ff0000\""));
    }

    fn para(runs: Vec<Run>) -> Paragraph {
        Paragraph { runs, ..Default::default() }
    }

    fn link_style(url: &str) -> CharStyle {
        CharStyle { link: Some(url.into()), ..Default::default() }
    }

    fn cell(paragraphs: Vec<Paragraph>) -> TableCell {
        TableCell { paragraphs }
    }

    fn table_doc() -> Document {
        let bold = CharStyle { bold: true, ..Default::default() };
        let bold_link = CharStyle { bold: true, link: Some("https://a.example/?x=1&y=<2>".into()), ..Default::default() };
        let t = Table {
            rows: vec![
                TableRow {
                    cells: vec![
                        cell(vec![para(vec![run("gras", bold.clone()), Run::plain(" et  normal")])]),
                        cell(vec![]),
                        cell(vec![
                            Paragraph { list: Some(ListKind::Bullet), runs: vec![Run::plain("l1")], ..Default::default() },
                            Paragraph { list: Some(ListKind::Bullet), runs: vec![Run::plain("l2")], ..Default::default() },
                            Paragraph { list: Some(ListKind::Numbered), runs: vec![Run::plain("n1")], ..Default::default() },
                            para(vec![Run::plain("apr\u{e8}s")]),
                        ]),
                    ],
                },
                TableRow {
                    cells: vec![
                        cell(vec![para(vec![
                            Run::plain("voir "),
                            run("lien", link_style("http://x.org")),
                            run("gras", bold_link.clone()),
                            run("suite", link_style("http://y.org")),
                            run("fin", bold.clone()),
                        ])]),
                        cell(vec![Paragraph::default()]),
                        cell(vec![
                            Paragraph { style: ParagraphStyle::Heading1, alignment: Alignment::Right, runs: vec![Run::plain("T")], ..Default::default() },
                            Paragraph::default(),
                        ]),
                    ],
                },
            ],
        };
        Document {
            metadata: Metadata::default(),
            blocks: vec![
                Block::Paragraph(para(vec![
                    run("a", link_style("http://top.org")),
                    run(" b ", link_style("http://top.org")),
                    run("c", bold.clone()),
                ])),
                Block::Table(t),
                Block::Table(Table::new(1, 1)),
                Block::Paragraph(Paragraph { list: Some(ListKind::Numbered), runs: vec![Run::plain("z")], ..Default::default() }),
            ],
        }
    }

    #[test]
    fn table_and_link_markup() {
        let bytes = write(&table_doc()).unwrap();
        for n in ["content.xml", "styles.xml"] {
            well_formed(&read_entry(&bytes, n));
        }
        let c = read_entry(&bytes, "content.xml");
        assert_eq!(c.matches("<table:table ").count(), 2);
        assert!(c.contains("table:name=\"Tableau1\" table:style-name=\"Tableau1\""));
        assert!(c.contains("table:name=\"Tableau2\""));
        assert!(c.contains("table:number-columns-repeated=\"3\""));
        assert_eq!(c.matches("<table:table-row>").count(), 3);
        assert_eq!(c.matches("<table:table-cell ").count(), 7);
        assert!(c.contains("style:family=\"table\""));
        assert!(c.contains("style:family=\"table-column\""));
        assert!(c.contains("fo:border=\"0.5pt solid #000000\""));
        assert!(c.contains("fo:padding=\"0.1cm\""));
        // Consecutive runs with the same link share one anchor, spans stay inside.
        assert!(c.contains("<text:a xlink:type=\"simple\" xlink:href=\"http://top.org\">a b </text:a>"));
        assert!(c.contains("xlink:href=\"https://a.example/?x=1&amp;y=&lt;2&gt;\"><text:span"));
        assert_eq!(c.matches("</text:a>").count(), c.matches("<text:a ").count());
    }

    #[test]
    fn roundtrip_tables_and_links() {
        let doc = table_doc();
        let bytes = write(&doc).unwrap();
        let back = crate::formats::odt_reader::read(&bytes).unwrap();
        assert_eq!(back.normalized(), doc.normalized());
    }

    #[test]
    fn roundtrip_table_only_and_empty_cells() {
        let doc = Document { metadata: Metadata::default(), blocks: vec![Block::Table(Table::new(3, 2))] };
        let back = crate::formats::odt_reader::read(&write(&doc).unwrap()).unwrap();
        assert_eq!(back.normalized(), doc.normalized());
        assert_eq!(back.blocks.len(), 1);
    }
}
