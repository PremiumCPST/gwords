//! DOCX (WordprocessingML) writer.

use std::fmt::Write as _;
use std::io::Write as _;

use zip::write::SimpleFileOptions;

use crate::document::{
    Alignment, Block, CharStyle, Document, ListKind, Paragraph, ParagraphStyle, Run, Table,
    VerticalAlign,
};
use crate::formats::FormatError;

const NS_W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const NS_R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
/// Usable text width of an A4 page with 2 cm margins, in twips.
const TEXT_WIDTH: u32 = 9638;
/// Relationship ids 1..=4 are taken by the fixed parts.
const FIRST_LINK_RID: usize = 5;
const XML_DECL: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";
const BULLET_NUM: u32 = 1;
const FIRST_NUMBERED_NUM: u32 = 2;

/// Serializes the document as a .docx archive.
pub fn write(doc: &Document) -> Result<Vec<u8>, FormatError> {
    let (document_xml, numbered_groups, links) = document_xml(doc);
    let now = iso_now();

    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let parts: Vec<(&str, String)> = vec![
        ("[Content_Types].xml", content_types_xml()),
        ("_rels/.rels", root_rels_xml()),
        ("word/document.xml", document_xml),
        ("word/_rels/document.xml.rels", document_rels_xml(&links)),
        ("word/styles.xml", styles_xml()),
        ("word/numbering.xml", numbering_xml(numbered_groups)),
        ("word/settings.xml", settings_xml()),
        ("word/fontTable.xml", font_table_xml()),
        ("docProps/core.xml", core_xml(doc, &now)),
        ("docProps/app.xml", app_xml()),
    ];
    for (name, content) in parts {
        zip.start_file(name, opts)?;
        zip.write_all(content.as_bytes())?;
    }
    Ok(zip.finish()?.into_inner())
}

// ---------------------------------------------------------------- escaping

fn is_xml_char(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r')
        || (c >= ' ' && c != '\u{FFFE}' && c != '\u{FFFF}')
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c if is_xml_char(c) => out.push(c),
            _ => {}
        }
    }
    out
}

// ---------------------------------------------------------------- time

fn iso_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    // Civil-from-days (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        y,
        m,
        d,
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

// ---------------------------------------------------------------- static parts

fn content_types_xml() -> String {
    format!(
        "{XML_DECL}<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
<Default Extension=\"xml\" ContentType=\"application/xml\"/>\
<Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/>\
<Override PartName=\"/word/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml\"/>\
<Override PartName=\"/word/numbering.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml\"/>\
<Override PartName=\"/word/settings.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml\"/>\
<Override PartName=\"/word/fontTable.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.fontTable+xml\"/>\
<Override PartName=\"/docProps/core.xml\" ContentType=\"application/vnd.openxmlformats-package.core-properties+xml\"/>\
<Override PartName=\"/docProps/app.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.extended-properties+xml\"/>\
</Types>"
    )
}

fn root_rels_xml() -> String {
    format!(
        "{XML_DECL}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/>\
<Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties\" Target=\"docProps/core.xml\"/>\
<Relationship Id=\"rId3\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties\" Target=\"docProps/app.xml\"/>\
</Relationships>"
    )
}

fn document_rels_xml(links: &[String]) -> String {
    let base = NS_R;
    let mut extra = String::new();
    for (i, url) in links.iter().enumerate() {
        let _ = write!(
            extra,
            "<Relationship Id=\"rId{}\" Type=\"{base}/hyperlink\" Target=\"{}\" TargetMode=\"External\"/>",
            FIRST_LINK_RID + i,
            escape(url)
        );
    }
    format!(
        "{XML_DECL}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"{base}/styles\" Target=\"styles.xml\"/>\
<Relationship Id=\"rId2\" Type=\"{base}/numbering\" Target=\"numbering.xml\"/>\
<Relationship Id=\"rId3\" Type=\"{base}/settings\" Target=\"settings.xml\"/>\
<Relationship Id=\"rId4\" Type=\"{base}/fontTable\" Target=\"fontTable.xml\"/>\
{extra}</Relationships>"
    )
}

fn settings_xml() -> String {
    format!(
        "{XML_DECL}<w:settings xmlns:w=\"{NS_W}\"><w:defaultTabStop w:val=\"708\"/>\
<w:compat><w:compatSetting w:name=\"compatibilityMode\" w:uri=\"http://schemas.microsoft.com/office/word\" w:val=\"15\"/></w:compat>\
</w:settings>"
    )
}

fn font_table_xml() -> String {
    format!(
        "{XML_DECL}<w:fonts xmlns:w=\"{NS_W}\">\
<w:font w:name=\"Cantarell\"><w:family w:val=\"swiss\"/><w:pitch w:val=\"variable\"/></w:font>\
<w:font w:name=\"Liberation Mono\"><w:family w:val=\"modern\"/><w:pitch w:val=\"fixed\"/></w:font>\
</w:fonts>"
    )
}

fn core_xml(doc: &Document, now: &str) -> String {
    let title = doc.metadata.title.as_deref().unwrap_or("");
    let author = doc.metadata.author.as_deref().unwrap_or("");
    format!(
        "{XML_DECL}<cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" \
xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:dcterms=\"http://purl.org/dc/terms/\" \
xmlns:dcmitype=\"http://purl.org/dc/dcmitype/\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">\
<dc:title>{}</dc:title><dc:creator>{}</dc:creator>\
<dcterms:created xsi:type=\"dcterms:W3CDTF\">{now}</dcterms:created>\
<dcterms:modified xsi:type=\"dcterms:W3CDTF\">{now}</dcterms:modified>\
</cp:coreProperties>",
        escape(title),
        escape(author)
    )
}

fn app_xml() -> String {
    format!(
        "{XML_DECL}<Properties xmlns=\"http://schemas.openxmlformats.org/officeDocument/2006/extended-properties\">\
<Application>Gwords</Application></Properties>"
    )
}

fn styles_xml() -> String {
    let heading = |id: &str, name: &str, size: u32, lvl: u32| {
        format!(
            "<w:style w:type=\"paragraph\" w:styleId=\"{id}\"><w:name w:val=\"{name}\"/>\
<w:basedOn w:val=\"Normal\"/><w:next w:val=\"Normal\"/><w:qFormat/>\
<w:pPr><w:keepNext/><w:spacing w:before=\"240\" w:after=\"120\"/><w:outlineLvl w:val=\"{lvl}\"/></w:pPr>\
<w:rPr><w:b/><w:bCs/><w:sz w:val=\"{size}\"/><w:szCs w:val=\"{size}\"/></w:rPr></w:style>"
        )
    };
    let mut s = String::new();
    s.push_str(XML_DECL);
    let _ = write!(
        s,
        "<w:styles xmlns:w=\"{NS_W}\"><w:docDefaults><w:rPrDefault><w:rPr>\
<w:rFonts w:ascii=\"Cantarell\" w:hAnsi=\"Cantarell\" w:eastAsia=\"Cantarell\" w:cs=\"Cantarell\"/>\
<w:sz w:val=\"24\"/><w:szCs w:val=\"24\"/><w:lang w:val=\"fr-FR\" w:eastAsia=\"fr-FR\" w:bidi=\"ar-SA\"/>\
</w:rPr></w:rPrDefault><w:pPrDefault><w:pPr><w:spacing w:after=\"120\" w:line=\"276\" w:lineRule=\"auto\"/></w:pPr></w:pPrDefault>\
</w:docDefaults>\
<w:style w:type=\"paragraph\" w:default=\"1\" w:styleId=\"Normal\"><w:name w:val=\"Normal\"/><w:qFormat/></w:style>\
<w:style w:type=\"character\" w:default=\"1\" w:styleId=\"DefaultParagraphFont\"><w:name w:val=\"Default Paragraph Font\"/><w:uiPriority w:val=\"1\"/><w:semiHidden/><w:unhideWhenUsed/></w:style>\
<w:style w:type=\"paragraph\" w:styleId=\"Title\"><w:name w:val=\"Title\"/><w:basedOn w:val=\"Normal\"/><w:next w:val=\"Normal\"/><w:qFormat/>\
<w:pPr><w:keepNext/><w:spacing w:before=\"0\" w:after=\"240\"/></w:pPr>\
<w:rPr><w:b/><w:bCs/><w:sz w:val=\"52\"/><w:szCs w:val=\"52\"/></w:rPr></w:style>"
    );
    s.push_str(&heading("Heading1", "heading 1", 40, 0));
    s.push_str(&heading("Heading2", "heading 2", 32, 1));
    s.push_str(&heading("Heading3", "heading 3", 28, 2));
    s.push_str(
        &"<w:style w:type=\"paragraph\" w:styleId=\"Quote\"><w:name w:val=\"Quote\"/><w:basedOn w:val=\"Normal\"/><w:next w:val=\"Normal\"/><w:qFormat/>\
<w:pPr><w:ind w:left=\"720\" w:right=\"720\"/></w:pPr><w:rPr><w:i/><w:iCs/></w:rPr></w:style>\
<w:style w:type=\"paragraph\" w:customStyle=\"1\" w:styleId=\"Code\"><w:name w:val=\"Code\"/><w:basedOn w:val=\"Normal\"/><w:qFormat/>\
<w:pPr><w:spacing w:after=\"0\" w:line=\"240\" w:lineRule=\"auto\"/></w:pPr>\
<w:rPr><w:rFonts w:ascii=\"Liberation Mono\" w:hAnsi=\"Liberation Mono\" w:eastAsia=\"Liberation Mono\" w:cs=\"Liberation Mono\"/><w:sz w:val=\"22\"/><w:szCs w:val=\"22\"/></w:rPr></w:style>\
<w:style w:type=\"paragraph\" w:styleId=\"ListParagraph\"><w:name w:val=\"List Paragraph\"/><w:basedOn w:val=\"Normal\"/><w:qFormat/>\
<w:pPr><w:ind w:left=\"720\"/><w:contextualSpacing/></w:pPr></w:style>\
<w:style w:type=\"character\" w:styleId=\"Hyperlink\"><w:name w:val=\"Hyperlink\"/><w:basedOn w:val=\"DefaultParagraphFont\"/><w:uiPriority w:val=\"99\"/><w:unhideWhenUsed/>\
<w:rPr><w:color w:val=\"0563C1\"/><w:u w:val=\"single\"/></w:rPr></w:style>\
<w:style w:type=\"table\" w:default=\"1\" w:styleId=\"TableNormal\"><w:name w:val=\"Normal Table\"/><w:uiPriority w:val=\"99\"/><w:semiHidden/><w:unhideWhenUsed/>\
<w:tblPr><w:tblInd w:w=\"0\" w:type=\"dxa\"/><w:tblCellMar><w:top w:w=\"0\" w:type=\"dxa\"/><w:left w:w=\"108\" w:type=\"dxa\"/><w:bottom w:w=\"0\" w:type=\"dxa\"/><w:right w:w=\"108\" w:type=\"dxa\"/></w:tblCellMar></w:tblPr></w:style>\
<w:style w:type=\"table\" w:styleId=\"TableGrid\"><w:name w:val=\"Table Grid\"/><w:basedOn w:val=\"TableNormal\"/><w:uiPriority w:val=\"39\"/>\
<w:pPr><w:spacing w:after=\"0\" w:line=\"240\" w:lineRule=\"auto\"/></w:pPr><w:tblPr><w:tblBorders>{TABLE_BORDERS}</w:tblBorders></w:tblPr></w:style>\
</w:styles>"
            .replace("{TABLE_BORDERS}", &table_borders()),
    );
    s
}

fn numbering_xml(numbered_groups: u32) -> String {
    let mut s = String::new();
    s.push_str(XML_DECL);
    let _ = write!(
        s,
        "<w:numbering xmlns:w=\"{NS_W}\">\
<w:abstractNum w:abstractNumId=\"0\"><w:multiLevelType w:val=\"hybridMultilevel\"/>\
<w:lvl w:ilvl=\"0\"><w:start w:val=\"1\"/><w:numFmt w:val=\"bullet\"/><w:lvlText w:val=\"\u{2022}\"/><w:lvlJc w:val=\"left\"/>\
<w:pPr><w:ind w:left=\"720\" w:hanging=\"360\"/></w:pPr></w:lvl></w:abstractNum>\
<w:abstractNum w:abstractNumId=\"1\"><w:multiLevelType w:val=\"hybridMultilevel\"/>\
<w:lvl w:ilvl=\"0\"><w:start w:val=\"1\"/><w:numFmt w:val=\"decimal\"/><w:lvlText w:val=\"%1.\"/><w:lvlJc w:val=\"left\"/>\
<w:pPr><w:ind w:left=\"720\" w:hanging=\"360\"/></w:pPr></w:lvl></w:abstractNum>\
<w:num w:numId=\"{BULLET_NUM}\"><w:abstractNumId w:val=\"0\"/></w:num>\
<w:num w:numId=\"{FIRST_NUMBERED_NUM}\"><w:abstractNumId w:val=\"1\"/></w:num>"
    );
    // Extra groups (num ids 3, 4, ...) restart at 1.
    for g in 1..numbered_groups {
        let id = FIRST_NUMBERED_NUM + g;
        let _ = write!(
            s,
            "<w:num w:numId=\"{id}\"><w:abstractNumId w:val=\"1\"/>\
<w:lvlOverride w:ilvl=\"0\"><w:startOverride w:val=\"1\"/></w:lvlOverride></w:num>"
        );
    }
    s.push_str("</w:numbering>");
    s
}

// ---------------------------------------------------------------- document

fn style_id(style: ParagraphStyle) -> Option<&'static str> {
    match style {
        ParagraphStyle::Normal => None,
        ParagraphStyle::Title => Some("Title"),
        ParagraphStyle::Heading1 => Some("Heading1"),
        ParagraphStyle::Heading2 => Some("Heading2"),
        ParagraphStyle::Heading3 => Some("Heading3"),
        ParagraphStyle::Quote => Some("Quote"),
        ParagraphStyle::Code => Some("Code"),
    }
}

fn border_xml(sides: &[&str]) -> String {
    sides
        .iter()
        .map(|n| format!("<w:{n} w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"auto\"/>"))
        .collect()
}

fn table_borders() -> String {
    border_xml(&["top", "left", "bottom", "right", "insideH", "insideV"])
}

/// Mutable state shared while writing the body.
#[derive(Default)]
struct Ctx {
    /// Number of numbered-list groups so far.
    groups: u32,
    /// Hyperlink targets (without '#' anchors), in relationship order.
    links: Vec<String>,
}

impl Ctx {
    fn link_rid(&mut self, url: &str) -> String {
        let i = match self.links.iter().position(|u| u == url) {
            Some(i) => i,
            None => {
                self.links.push(url.to_string());
                self.links.len() - 1
            }
        };
        format!("rId{}", FIRST_LINK_RID + i)
    }
}

/// Returns the document.xml content, the number of numbered-list groups and the hyperlink targets.
fn document_xml(doc: &Document) -> (String, u32, Vec<String>) {
    let mut s = String::new();
    s.push_str(XML_DECL);
    let _ = write!(s, "<w:document xmlns:w=\"{NS_W}\" xmlns:r=\"{NS_R}\"><w:body>");
    let mut ctx = Ctx::default();
    let mut in_numbered = false;
    for b in &doc.blocks {
        match b {
            Block::Paragraph(p) => write_listed_paragraph(&mut s, &mut ctx, &mut in_numbered, p),
            Block::Table(t) => {
                in_numbered = false;
                write_table(&mut s, &mut ctx, t);
            }
        }
    }
    s.push_str(
        "<w:sectPr><w:pgSz w:w=\"11906\" w:h=\"16838\"/>\
<w:pgMar w:top=\"1134\" w:right=\"1134\" w:bottom=\"1134\" w:left=\"1134\" w:header=\"709\" w:footer=\"709\" w:gutter=\"0\"/>\
</w:sectPr></w:body></w:document>",
    );
    (s, ctx.groups, ctx.links)
}

fn write_listed_paragraph(s: &mut String, ctx: &mut Ctx, in_numbered: &mut bool, p: &Paragraph) {
    if p.list == Some(ListKind::Numbered) {
        if !*in_numbered {
            ctx.groups += 1;
        }
        *in_numbered = true;
    } else {
        *in_numbered = false;
    }
    let num_id = match p.list {
        None => None,
        Some(ListKind::Bullet) => Some(BULLET_NUM),
        Some(ListKind::Numbered) => Some(FIRST_NUMBERED_NUM + ctx.groups - 1),
    };
    write_paragraph(s, ctx, p, num_id);
}

fn write_table(s: &mut String, ctx: &mut Ctx, table: &Table) {
    let mut t = table.clone();
    t.make_rectangular();
    let cols = t.column_count();
    if t.rows.is_empty() || cols == 0 {
        return;
    }
    let cols32 = cols as u32;
    let _ = write!(
        s,
        "<w:tbl><w:tblPr><w:tblStyle w:val=\"TableGrid\"/><w:tblW w:w=\"5000\" w:type=\"pct\"/>\
<w:tblBorders>{}</w:tblBorders>\
<w:tblLook w:val=\"04A0\" w:firstRow=\"1\" w:lastRow=\"0\" w:firstColumn=\"1\" w:lastColumn=\"0\" w:noHBand=\"0\" w:noVBand=\"1\"/></w:tblPr><w:tblGrid>",
        table_borders()
    );
    for _ in 0..cols {
        let _ = write!(s, "<w:gridCol w:w=\"{}\"/>", TEXT_WIDTH / cols32);
    }
    s.push_str("</w:tblGrid>");
    for row in &t.rows {
        s.push_str("<w:tr>");
        for cell in &row.cells {
            let _ = write!(s, "<w:tc><w:tcPr><w:tcW w:w=\"{}\" w:type=\"pct\"/></w:tcPr>", 5000 / cols32);
            let mut in_numbered = false;
            for p in &cell.paragraphs {
                write_listed_paragraph(s, ctx, &mut in_numbered, p);
            }
            if cell.paragraphs.is_empty() {
                s.push_str("<w:p/>");
            }
            s.push_str("</w:tc>");
        }
        s.push_str("</w:tr>");
    }
    s.push_str("</w:tbl>");
}

fn write_paragraph(s: &mut String, ctx: &mut Ctx, p: &Paragraph, num_id: Option<u32>) {
    s.push_str("<w:p>");
    let sid = style_id(p.style).or(if num_id.is_some() { Some("ListParagraph") } else { None });
    let jc = match p.alignment {
        Alignment::Left => None,
        Alignment::Center => Some("center"),
        Alignment::Right => Some("right"),
        Alignment::Justify => Some("both"),
    };
    if sid.is_some() || num_id.is_some() || jc.is_some() {
        s.push_str("<w:pPr>");
        if let Some(id) = sid {
            let _ = write!(s, "<w:pStyle w:val=\"{id}\"/>");
        }
        if let Some(n) = num_id {
            let _ = write!(s, "<w:numPr><w:ilvl w:val=\"0\"/><w:numId w:val=\"{n}\"/></w:numPr>");
        }
        if let Some(j) = jc {
            let _ = write!(s, "<w:jc w:val=\"{j}\"/>");
        }
        s.push_str("</w:pPr>");
    }
    let runs: Vec<&Run> = p.runs.iter().filter(|r| !r.text.is_empty()).collect();
    let mut i = 0;
    while i < runs.len() {
        let link = runs[i].style.link.as_deref();
        let mut j = i + 1;
        while link.is_some() && j < runs.len() && runs[j].style.link.as_deref() == link {
            j += 1;
        }
        if let Some(url) = link {
            match url.strip_prefix('#').filter(|a| !a.is_empty()) {
                Some(anchor) => {
                    let _ = write!(s, "<w:hyperlink w:anchor=\"{}\" w:history=\"1\">", escape(anchor));
                }
                None => {
                    let rid = ctx.link_rid(url);
                    let _ = write!(s, "<w:hyperlink r:id=\"{rid}\" w:history=\"1\">");
                }
            }
        }
        for r in &runs[i..j] {
            write_run(s, r);
        }
        if link.is_some() {
            s.push_str("</w:hyperlink>");
        }
        i = j;
    }
    s.push_str("</w:p>");
}

fn rpr_xml(st: &CharStyle) -> String {
    let mut s = String::new();
    if st.link.is_some() {
        s.push_str("<w:rStyle w:val=\"Hyperlink\"/>");
    }
    if let Some(f) = st.font_family.as_deref().filter(|f| !f.trim().is_empty()) {
        let f = escape(f);
        let _ = write!(
            s,
            "<w:rFonts w:ascii=\"{f}\" w:hAnsi=\"{f}\" w:eastAsia=\"{f}\" w:cs=\"{f}\"/>"
        );
    }
    if st.bold {
        s.push_str("<w:b/><w:bCs/>");
    }
    if st.italic {
        s.push_str("<w:i/><w:iCs/>");
    }
    if st.strikethrough {
        s.push_str("<w:strike/>");
    }
    if let Some(c) = st.color {
        let _ = write!(s, "<w:color w:val=\"{}\"/>", c.to_hex().to_uppercase());
    }
    if let Some(pt) = st.font_size.filter(|p| p.is_finite() && *p > 0.0) {
        let hp = ((pt * 2.0).round() as i64).clamp(1, 3276);
        let _ = write!(s, "<w:sz w:val=\"{hp}\"/><w:szCs w:val=\"{hp}\"/>");
    }
    if st.underline {
        s.push_str("<w:u w:val=\"single\"/>");
    }
    if let Some(c) = st.highlight {
        let _ = write!(
            s,
            "<w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"{}\"/>",
            c.to_hex().to_uppercase()
        );
    }
    match st.vertical {
        VerticalAlign::Baseline => {}
        VerticalAlign::Superscript => s.push_str("<w:vertAlign w:val=\"superscript\"/>"),
        VerticalAlign::Subscript => s.push_str("<w:vertAlign w:val=\"subscript\"/>"),
    }
    if s.is_empty() { s } else { format!("<w:rPr>{s}</w:rPr>") }
}

fn write_run(s: &mut String, run: &Run) {
    if run.text.is_empty() {
        return;
    }
    let rpr = rpr_xml(&run.style);
    let mut buf = String::new();
    let flush = |s: &mut String, buf: &mut String| {
        if buf.is_empty() {
            return;
        }
        let t = escape(buf);
        if !t.is_empty() {
            let _ = write!(s, "<w:r>{rpr}<w:t xml:space=\"preserve\">{t}</w:t></w:r>");
        }
        buf.clear();
    };
    for c in run.text.chars() {
        match c {
            '\t' => {
                flush(s, &mut buf);
                let _ = write!(s, "<w:r>{rpr}<w:tab/></w:r>");
            }
            '\u{2028}' => {
                flush(s, &mut buf);
                let _ = write!(s, "<w:r>{rpr}<w:br/></w:r>");
            }
            '\n' | '\r' => {
                flush(s, &mut buf);
                let _ = write!(s, "<w:r>{rpr}<w:br/></w:r>");
            }
            c => buf.push(c),
        }
    }
    flush(s, &mut buf);
}

// ---------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Metadata, Rgb, TableCell, TableRow};
    use std::io::Read;

    fn read_part(bytes: &[u8], name: &str) -> String {
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let mut f = z.by_name(name).unwrap_or_else(|_| panic!("missing {name}"));
        let mut s = String::new();
        f.read_to_string(&mut s).unwrap();
        s
    }

    fn well_formed(xml: &str) {
        let mut r = quick_xml::Reader::from_str(xml);
        loop {
            match r.read_event().unwrap() {
                quick_xml::events::Event::Eof => break,
                _ => {}
            }
        }
    }

    fn para(style: ParagraphStyle, list: Option<ListKind>, text: &str) -> Paragraph {
        Paragraph { style, list, runs: vec![Run::plain(text)], ..Default::default() }
    }

    #[test]
    fn parts_present_and_well_formed() {
        let doc = Document {
            metadata: Metadata { title: Some("A & B <t>".into()), author: Some("Moi".into()) },
            blocks: vec![para(ParagraphStyle::Heading1, None, "Salut").into()],
        };
        let bytes = write(&doc).unwrap();
        for name in [
            "[Content_Types].xml",
            "_rels/.rels",
            "word/document.xml",
            "word/_rels/document.xml.rels",
            "word/styles.xml",
            "word/numbering.xml",
            "word/settings.xml",
            "word/fontTable.xml",
            "docProps/core.xml",
            "docProps/app.xml",
        ] {
            well_formed(&read_part(&bytes, name));
        }
        let core = read_part(&bytes, "docProps/core.xml");
        assert!(core.contains("A &amp; B &lt;t&gt;"));
        assert!(read_part(&bytes, "word/document.xml").contains("w:val=\"Heading1\""));
    }

    #[test]
    fn rpr_order_and_escaping() {
        let style = CharStyle {
            bold: true,
            italic: true,
            underline: true,
            strikethrough: true,
            vertical: VerticalAlign::Superscript,
            font_family: Some("Foo\"Bar".into()),
            font_size: Some(10.5),
            color: Some(Rgb(255, 0, 0)),
            highlight: Some(Rgb(1, 2, 3)),
            link: None,
        };
        let doc = Document {
            blocks: vec![Paragraph {
                alignment: Alignment::Justify,
                runs: vec![Run { text: "a<b&c\u{1}\td\u{2028}e".into(), style }],
                ..Default::default()
            }
            .into()],
            ..Default::default()
        };
        let bytes = write(&doc).unwrap();
        let x = read_part(&bytes, "word/document.xml");
        well_formed(&x);
        let order = ["w:rFonts", "w:b/", "w:i/", "w:strike", "w:color", "w:sz ", "w:szCs", "w:u ", "w:shd", "w:vertAlign"];
        let mut last = 0;
        for tag in order {
            let pos = x.find(&format!("<{tag}")).unwrap_or_else(|| panic!("no {tag}"));
            assert!(pos > last, "{tag} out of order");
            last = pos;
        }
        assert!(x.contains("a&lt;b&amp;c"));
        assert!(!x.contains('\u{1}'));
        assert!(x.contains("<w:tab/>") && x.contains("<w:br/>"));
        assert!(x.contains("w:jc w:val=\"both\""));
        assert!(x.contains("Foo&quot;Bar"));
        assert!(x.contains("w:val=\"21\""));
        assert!(x.contains("w:fill=\"010203\""));
    }

    #[test]
    fn numbering_restart_groups() {
        let n = |t| para(ParagraphStyle::Normal, Some(ListKind::Numbered), t);
        let doc = Document {
            blocks: vec![
                n("a").into(),
                n("b").into(),
                para(ParagraphStyle::Normal, Some(ListKind::Bullet), "x").into(),
                n("c").into(),
                para(ParagraphStyle::Normal, None, "gap").into(),
                n("d").into(),
            ],
            ..Default::default()
        };
        let bytes = write(&doc).unwrap();
        let x = read_part(&bytes, "word/document.xml");
        assert_eq!(x.matches("<w:numId w:val=\"2\"/>").count(), 2);
        assert_eq!(x.matches("<w:numId w:val=\"1\"/>").count(), 1);
        assert_eq!(x.matches("<w:numId w:val=\"3\"/>").count(), 1);
        assert_eq!(x.matches("<w:numId w:val=\"4\"/>").count(), 1);
        let num = read_part(&bytes, "word/numbering.xml");
        well_formed(&num);
        assert!(num.contains("w:numId=\"3\"") && num.contains("w:numId=\"4\""));
        assert_eq!(num.matches("<w:startOverride").count(), 2);
    }

    fn run(text: &str, style: CharStyle) -> Run {
        Run { text: text.into(), style }
    }

    fn linked(url: &str) -> CharStyle {
        CharStyle { link: Some(url.into()), ..Default::default() }
    }

    fn cell_of(paragraphs: Vec<Paragraph>) -> TableCell {
        TableCell { paragraphs }
    }

    fn sample_doc() -> Document {
        let bold = CharStyle { bold: true, ..Default::default() };
        let bold_link = CharStyle { bold: true, link: Some("https://a.org/x?y=1&z=2".into()), ..Default::default() };
        let rich = Paragraph {
            runs: vec![
                run("gras ", bold.clone()),
                run("lien", bold_link),
                run("lien2", linked("https://a.org/x?y=1&z=2")),
                run(" fin", CharStyle::default()),
                run("autre", linked("https://b.org")),
                run("interne", linked("#sec")),
            ],
            ..Default::default()
        };
        let table = Table {
            rows: vec![
                TableRow {
                    cells: vec![
                        cell_of(vec![rich.clone(), para(ParagraphStyle::Normal, Some(ListKind::Numbered), "un")]),
                        cell_of(vec![]),
                        cell_of(vec![para(ParagraphStyle::Heading2, Some(ListKind::Bullet), "puce")]),
                    ],
                },
                TableRow {
                    cells: vec![
                        cell_of(vec![Paragraph { alignment: Alignment::Center, runs: vec![run("c", bold)], ..Default::default() }]),
                        cell_of(vec![para(ParagraphStyle::Normal, Some(ListKind::Numbered), "deux")]),
                        cell_of(vec![]),
                    ],
                },
            ],
        };
        Document {
            metadata: Metadata::default(),
            blocks: vec![rich.into(), table.clone().into(), para(ParagraphStyle::Normal, None, "entre").into(), table.into(), para(ParagraphStyle::Normal, None, "fin").into()],
        }
    }

    #[test]
    fn tables_and_links_well_formed() {
        let bytes = write(&sample_doc()).unwrap();
        for name in ["word/document.xml", "word/styles.xml", "word/_rels/document.xml.rels", "word/numbering.xml"] {
            well_formed(&read_part(&bytes, name));
        }
        let x = read_part(&bytes, "word/document.xml");
        assert!(x.contains("<w:tblStyle w:val=\"TableGrid\"/>") && x.contains("w:type=\"pct\""));
        assert!(x.contains("<w:insideH") && x.contains("<w:insideV"));
        assert!(x.contains("<w:gridCol w:w=\"3212\"/>"));
        assert!(x.contains("<w:tc><w:tcPr><w:tcW w:w=\"1666\" w:type=\"pct\"/></w:tcPr><w:p/></w:tc>"));
        assert!(x.contains("w:anchor=\"sec\""));
        // Same URL: one relationship, shared across runs and tables.
        let rels = read_part(&bytes, "word/_rels/document.xml.rels");
        assert_eq!(rels.matches("/hyperlink\"").count(), 2, "{rels}");
        assert!(rels.contains("TargetMode=\"External\"") && rels.contains("y=1&amp;z=2"));
        let styles = read_part(&bytes, "word/styles.xml");
        assert!(styles.contains("w:styleId=\"TableGrid\"") && styles.contains("w:styleId=\"Hyperlink\"") && styles.contains("0563C1"));
        // rStyle must come first in rPr.
        assert!(x.contains("<w:rPr><w:rStyle w:val=\"Hyperlink\"/><w:b/><w:bCs/></w:rPr>"));
    }

    #[test]
    fn round_trip_tables_and_links() {
        let doc = sample_doc();
        let back = crate::formats::docx_reader::read(&write(&doc).unwrap()).unwrap();
        assert_eq!(back.normalized(), doc.normalized());
    }
}
