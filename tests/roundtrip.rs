//! Integration tests: round-trips through the public format API.
//!
//! These tests do not need GTK. They exercise `gwords::formats` (ODT, DOCX,
//! plain text) against the shared `gwords::document` model.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use gwords::document::{
    Alignment, Block, CharStyle, Document, ListKind, Metadata, Paragraph, ParagraphStyle, Rgb, Run,
    Table, TableCell, TableRow, VerticalAlign,
};
use gwords::formats::{load, read_bytes, save, write_bytes, FileFormat};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn plain(text: &str) -> Run {
    Run::plain(text)
}

fn styled(text: &str, configure: impl FnOnce(&mut CharStyle)) -> Run {
    let mut style = CharStyle::default();
    configure(&mut style);
    Run { text: text.to_string(), style }
}

fn para(style: ParagraphStyle, runs: Vec<Run>) -> Paragraph {
    Paragraph { style, runs, ..Default::default() }
}

fn aligned(alignment: Alignment, runs: Vec<Run>) -> Paragraph {
    Paragraph { alignment, runs, ..Default::default() }
}

fn list_item(kind: ListKind, text: &str) -> Paragraph {
    Paragraph { list: Some(kind), runs: vec![plain(text)], ..Default::default() }
}

fn cell(paragraphs: Vec<Paragraph>) -> TableCell {
    TableCell { paragraphs }
}

fn table(rows: Vec<Vec<TableCell>>) -> Block {
    Block::Table(Table {
        rows: rows.into_iter().map(|cells| TableRow { cells }).collect(),
    })
}

/// A link to `url` with the given text, optionally bold.
fn link(text: &str, url: &str, bold: bool) -> Run {
    styled(text, |s| {
        s.link = Some(url.to_string());
        s.bold = bold;
    })
}

/// Covers every feature the format readers and writers must preserve.
fn sample_doc() -> Document {
    let metadata = Metadata {
        title: Some("Document d'essai <&> \"é\"".to_string()),
        author: Some("Zoé Dupont".to_string()),
    };

    let mut paragraphs = Vec::new();

    paragraphs.push(para(ParagraphStyle::Title, vec![plain("Titre du document")]));
    paragraphs.push(para(ParagraphStyle::Heading1, vec![plain("Chapitre 1")]));
    paragraphs.push(para(ParagraphStyle::Heading2, vec![plain("Section 1.1")]));
    paragraphs.push(para(ParagraphStyle::Heading3, vec![plain("Sous-section 1.1.1")]));

    // Character attributes: bold, italic, underline, strike, super/subscript.
    paragraphs.push(para(
        ParagraphStyle::Normal,
        vec![
            plain("Texte normal, "),
            styled("gras", |s| s.bold = true),
            styled(" italique", |s| s.italic = true),
            styled(" souligné", |s| s.underline = true),
            styled(" barré", |s| s.strikethrough = true),
            plain(" x"),
            styled("2", |s| s.vertical = VerticalAlign::Superscript),
            plain(" H"),
            styled("2", |s| s.vertical = VerticalAlign::Subscript),
            plain("O."),
        ],
    ));

    // Font family, size, color, highlight, and a combination of all of them.
    paragraphs.push(para(
        ParagraphStyle::Normal,
        vec![
            styled("Police serif", |s| {
                s.font_family = Some("Liberation Serif".to_string());
                s.font_size = Some(18.0);
            }),
            styled(" couleur", |s| s.color = Some(Rgb(0xc0, 0x1c, 0x28))),
            styled(" surlignage", |s| s.highlight = Some(Rgb(0xf6, 0xd3, 0x2d))),
            styled(" tout ensemble", |s| {
                s.bold = true;
                s.italic = true;
                s.underline = true;
                s.strikethrough = true;
                s.font_family = Some("Liberation Serif".to_string());
                s.font_size = Some(18.0);
                s.color = Some(Rgb(0xc0, 0x1c, 0x28));
                s.highlight = Some(Rgb(0xf6, 0xd3, 0x2d));
            }),
        ],
    ));

    // Alignments.
    paragraphs.push(aligned(
        Alignment::Center,
        vec![plain("Centré : unicode é à ç œ 日本 🎉")],
    ));
    paragraphs.push(aligned(
        Alignment::Right,
        vec![plain("Aligné à droite : < > & \" ' </b>")],
    ));
    paragraphs.push(aligned(
        Alignment::Justify,
        vec![plain(
            "Justifié : ce paragraphe est assez long pour occuper plusieurs lignes \
             et vérifier que l'alignement justifié survit au passage par le format.",
        )],
    ));

    paragraphs.push(para(ParagraphStyle::Quote, vec![plain("Une citation célèbre.")]));

    // Code: consecutive spaces, a tab, and trailing spaces.
    paragraphs.push(para(
        ParagraphStyle::Code,
        vec![plain("fn  main() {\tprintln!(\"a    b\");   }   ")],
    ));

    // Soft line break (U+2028) inside a paragraph.
    paragraphs.push(para(
        ParagraphStyle::Normal,
        vec![plain("Ligne un\u{2028}Ligne deux après un saut doux.")],
    ));

    // Bullet list followed by a numbered list.
    paragraphs.push(list_item(ListKind::Bullet, "Puce un"));
    paragraphs.push(list_item(ListKind::Bullet, "Puce deux"));
    paragraphs.push(list_item(ListKind::Bullet, "Puce trois"));
    paragraphs.push(list_item(ListKind::Numbered, "Premier"));
    paragraphs.push(list_item(ListKind::Numbered, "Second"));

    // Empty paragraph, then a closing paragraph.
    paragraphs.push(para(ParagraphStyle::Normal, vec![]));
    paragraphs.push(para(ParagraphStyle::Normal, vec![plain("Fin du document.")]));

    Document {
        metadata,
        blocks: paragraphs.into_iter().map(Block::Paragraph).collect(),
    }
}

/// A paragraph, a 3x3 table (bold header, a two-item bullet list, an empty
/// cell), then a closing paragraph.
fn table_doc() -> Document {
    let bullets = cell(vec![
        list_item(ListKind::Bullet, "Premier point"),
        list_item(ListKind::Bullet, "Second point"),
    ]);
    let header = |text: &str| cell(vec![para(ParagraphStyle::Normal, vec![styled(text, |s| s.bold = true)])]);
    Document {
        metadata: Metadata::default(),
        blocks: vec![
            Block::Paragraph(para(ParagraphStyle::Normal, vec![plain("Avant le tableau.")])),
            table(vec![
                vec![header("Nom"), header("Quantité"), header("Prix")],
                vec![
                    cell(vec![para(ParagraphStyle::Normal, vec![plain("Pomme")])]),
                    cell(vec![para(ParagraphStyle::Normal, vec![plain("3")])]),
                    cell(vec![para(ParagraphStyle::Normal, vec![plain("1,20 €")])]),
                ],
                vec![bullets, TableCell::default(), cell(vec![para(ParagraphStyle::Normal, vec![plain("fin")])])],
            ]),
            Block::Paragraph(para(ParagraphStyle::Normal, vec![plain("Après le tableau.")])),
        ],
    }
}

/// Paragraph and table cell links: plain, bold, and inside a table.
fn link_doc() -> Document {
    const GNOME: &str = "https://www.gnome.org/?a=1&b=2";
    Document {
        metadata: Metadata::default(),
        blocks: vec![
            Block::Paragraph(Paragraph {
                runs: vec![plain("Voir "), link("le site", GNOME, false), plain(" ici")],
                ..Default::default()
            }),
            Block::Paragraph(Paragraph {
                runs: vec![link("lien gras", GNOME, true)],
                ..Default::default()
            }),
            table(vec![vec![
                cell(vec![Paragraph {
                    runs: vec![plain("Cellule "), link("liée", GNOME, false)],
                    ..Default::default()
                }]),
                TableCell::default(),
            ]]),
        ],
    }
}

/// Asserts that metadata and blocks match, after normalization.
/// Reports the first differing block by index, with expected and got.
fn assert_same_document(expected: &Document, got: &Document, ctx: &str) {
    assert_eq!(got.metadata, expected.metadata, "{ctx}: metadata differs");

    let expected = expected.normalized();
    let got = got.normalized();

    assert_eq!(
        got.blocks.len(),
        expected.blocks.len(),
        "{ctx}: block count differs"
    );
    for (i, (e, g)) in expected.blocks.iter().zip(&got.blocks).enumerate() {
        assert_eq!(
            g, e,
            "{ctx}: block {i} differs\n  expected: {e:?}\n  got:      {g:?}"
        );
    }
}

/// Plain text with trailing empty paragraphs removed, so that the trailing
/// newline convention does not matter.
fn trimmed_plain_text(doc: &Document) -> String {
    let mut lines: Vec<String> = doc.paragraphs().map(Paragraph::plain_text).collect();
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    lines.join("\n")
}

/// Deletes the wrapped file when dropped, so tests clean up even on failure.
struct TempFile(PathBuf);

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn temp_file(ext: &str) -> TempFile {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before 1970")
        .as_nanos();
    let name = format!("gwords-roundtrip-{}-{nanos}.{ext}", std::process::id());
    TempFile(std::env::temp_dir().join(name))
}

fn roundtrip(doc: &Document, format: FileFormat) -> Document {
    let bytes = write_bytes(doc, format)
        .unwrap_or_else(|e| panic!("write {format:?} failed: {e}"));
    read_bytes(&bytes, format).unwrap_or_else(|e| panic!("read {format:?} failed: {e}"))
}

// ---------------------------------------------------------------------------
// Round-trips
// ---------------------------------------------------------------------------

#[test]
fn odt_roundtrip() {
    let sample = sample_doc();
    let got = roundtrip(&sample, FileFormat::Odt);
    assert_same_document(&sample, &got, "odt");
}

#[test]
fn docx_roundtrip() {
    let sample = sample_doc();
    let got = roundtrip(&sample, FileFormat::Docx);
    assert_same_document(&sample, &got, "docx");
}

fn metadata_roundtrip(format: FileFormat) {
    let doc = Document {
        metadata: Metadata {
            title: Some("Titre <é> & \"q\" 'a'".to_string()),
            author: Some("Zoé Dupont".to_string()),
        },
        blocks: vec![Block::Paragraph(para(ParagraphStyle::Normal, vec![plain("Corps")]))],
    };
    let got = roundtrip(&doc, format);
    assert_eq!(got.metadata.title, doc.metadata.title, "{format:?}: title differs");
    assert_eq!(got.metadata.author, doc.metadata.author, "{format:?}: author differs");
}

#[test]
fn odt_metadata_roundtrip() {
    metadata_roundtrip(FileFormat::Odt);
}

#[test]
fn docx_metadata_roundtrip() {
    metadata_roundtrip(FileFormat::Docx);
}

#[test]
fn cross_format_docx_to_odt() {
    let sample = sample_doc();

    let docx_bytes = write_bytes(&sample, FileFormat::Docx).expect("write docx");
    let doc_from_docx = read_bytes(&docx_bytes, FileFormat::Docx).expect("read docx");

    let odt_bytes = write_bytes(&doc_from_docx, FileFormat::Odt).expect("write odt");
    let doc_from_odt = read_bytes(&odt_bytes, FileFormat::Odt).expect("read odt");

    assert_same_document(&doc_from_docx, &doc_from_odt, "docx -> odt");
    assert_same_document(&sample, &doc_from_odt, "sample -> docx -> odt");
}

#[test]
fn table_roundtrip_odt() {
    let doc = table_doc();
    let got = roundtrip(&doc, FileFormat::Odt);
    assert_same_document(&doc, &got, "table odt");
}

#[test]
fn table_roundtrip_docx() {
    let doc = table_doc();
    let got = roundtrip(&doc, FileFormat::Docx);
    assert_same_document(&doc, &got, "table docx");
}

#[test]
fn link_roundtrip_odt() {
    let doc = link_doc();
    let got = roundtrip(&doc, FileFormat::Odt);
    assert_same_document(&doc, &got, "link odt");
}

#[test]
fn link_roundtrip_docx() {
    let doc = link_doc();
    let got = roundtrip(&doc, FileFormat::Docx);
    assert_same_document(&doc, &got, "link docx");
}

#[test]
fn cross_format_with_tables() {
    let doc = table_doc();

    let docx_bytes = write_bytes(&doc, FileFormat::Docx).expect("write docx");
    let doc_from_docx = read_bytes(&docx_bytes, FileFormat::Docx).expect("read docx");

    let odt_bytes = write_bytes(&doc_from_docx, FileFormat::Odt).expect("write odt");
    let doc_from_odt = read_bytes(&odt_bytes, FileFormat::Odt).expect("read odt");

    assert_same_document(&doc_from_docx, &doc_from_odt, "table docx -> odt");
    assert_same_document(&doc, &doc_from_odt, "table sample -> docx -> odt");
}

#[test]
fn plain_writes_tables_as_tsv() {
    let doc = table_doc();
    let bytes = write_bytes(&doc, FileFormat::PlainText).expect("write plain");
    let out = String::from_utf8(bytes).expect("utf-8 output");
    assert_eq!(
        out,
        "Avant le tableau.\n\
         Nom\tQuantité\tPrix\n\
         Pomme\t3\t1,20 €\n\
         Premier point Second point\t\tfin\n\
         Après le tableau.\n"
    );
}

#[test]
fn plain_roundtrip() {
    let text = "Première ligne é 日本\n\nTroisième & <ligne>\nDernière 🎉\n";
    let doc = Document::from_plain_text(text);

    let bytes = write_bytes(&doc, FileFormat::PlainText).expect("write plain");
    assert!(bytes.ends_with(b"\n"), "plain writer should end with a newline");

    let got = read_bytes(&bytes, FileFormat::PlainText).expect("read plain");
    assert_eq!(
        trimmed_plain_text(&got),
        trimmed_plain_text(&doc),
        "plain text differs after roundtrip"
    );
}

fn empty_document_roundtrip(format: FileFormat) {
    let got = roundtrip(&Document::default(), format);
    assert!(
        got.blocks.len() <= 1,
        "{format:?}: empty document read back with {} blocks",
        got.blocks.len()
    );
    assert_eq!(got.plain_text(), "", "{format:?}: empty document has text after roundtrip");
}

#[test]
fn empty_document_odt() {
    empty_document_roundtrip(FileFormat::Odt);
}

#[test]
fn empty_document_docx() {
    empty_document_roundtrip(FileFormat::Docx);
}

#[test]
fn empty_document_plain() {
    empty_document_roundtrip(FileFormat::PlainText);
}

#[test]
fn large_document_roundtrip_is_fast() {
    let paragraphs: Vec<Paragraph> = (0..5_000)
        .map(|i| {
            if i % 10 == 0 {
                para(ParagraphStyle::Heading1, vec![plain(&format!("Titre {i}"))])
            } else {
                para(
                    ParagraphStyle::Normal,
                    vec![
                        plain(&format!("Paragraphe {i} : texte é 日本 ")),
                        styled("gras", |s| s.bold = true),
                    ],
                )
            }
        })
        .collect();
    let doc = Document {
        metadata: Metadata::default(),
        blocks: paragraphs.into_iter().map(Block::Paragraph).collect(),
    };

    for format in [FileFormat::Docx, FileFormat::Odt] {
        let start = Instant::now();
        let got = roundtrip(&doc, format);
        let elapsed = start.elapsed();

        assert!(
            elapsed < Duration::from_secs(20),
            "{format:?}: 5000-paragraph roundtrip took {elapsed:?}"
        );
        assert_same_document(&doc, &got, &format!("large {format:?}"));
    }
}

// ---------------------------------------------------------------------------
// Files on disk
// ---------------------------------------------------------------------------

#[test]
fn save_and_load_odt_and_docx_files() {
    let sample = sample_doc();

    for format in [FileFormat::Odt, FileFormat::Docx] {
        let tmp = temp_file(format.extension());
        save(&sample, &tmp.0).unwrap_or_else(|e| panic!("save {format:?} failed: {e}"));

        let raw = std::fs::read(&tmp.0).expect("read saved file");
        assert!(raw.starts_with(b"PK"), "{format:?}: saved file is not a zip archive");

        let got = load(&tmp.0).unwrap_or_else(|e| panic!("load {format:?} failed: {e}"));
        assert_same_document(&sample, &got, &format!("file {format:?}"));
    }
}

#[test]
fn save_and_load_txt_file() {
    let doc = Document::from_plain_text("ligne 1\nligne 2 é 日本");
    let tmp = temp_file(FileFormat::PlainText.extension());

    save(&doc, &tmp.0).expect("save txt");

    let raw = std::fs::read_to_string(&tmp.0).expect("read saved txt");
    assert_eq!(raw.trim_end_matches('\n'), "ligne 1\nligne 2 é 日本");

    let got = load(&tmp.0).expect("load txt");
    assert_eq!(trimmed_plain_text(&got), trimmed_plain_text(&doc));
}

#[test]
fn load_missing_file_is_error() {
    let tmp = temp_file("odt");
    assert!(!tmp.0.exists(), "unique temp name unexpectedly exists");
    assert!(load(&tmp.0).is_err(), "loading a missing file should fail");
}

#[test]
fn format_detected_from_extension() {
    assert_eq!(FileFormat::from_path(Path::new("a.odt")), FileFormat::Odt);
    assert_eq!(FileFormat::from_path(Path::new("A.ODT")), FileFormat::Odt);
    assert_eq!(FileFormat::from_path(Path::new("b.docx")), FileFormat::Docx);
    assert_eq!(FileFormat::from_path(Path::new("c.txt")), FileFormat::PlainText);
    assert_eq!(FileFormat::from_path(Path::new("noext")), FileFormat::PlainText);
}

// ---------------------------------------------------------------------------
// Invalid input
// ---------------------------------------------------------------------------

#[test]
fn garbage_input_is_error_not_panic() {
    for format in [FileFormat::Odt, FileFormat::Docx] {
        assert!(
            read_bytes(b"not a zip", format).is_err(),
            "{format:?}: garbage bytes should be an error"
        );
        assert!(
            read_bytes(&[], format).is_err(),
            "{format:?}: empty input should be an error"
        );
    }
}

#[test]
fn truncated_archive_is_error_not_panic() {
    for format in [FileFormat::Odt, FileFormat::Docx] {
        let bytes = write_bytes(&sample_doc(), format).expect("write");
        let cut = &bytes[..bytes.len() / 2];
        assert!(
            read_bytes(cut, format).is_err(),
            "{format:?}: truncated archive should be an error"
        );
    }
}

/// Direct alignment on a styled paragraph (e.g. a centered Title) must survive,
/// even when the writer's named style already uses the same alignment.
#[test]
fn styled_paragraph_alignment_roundtrip() {
    for style in [ParagraphStyle::Title, ParagraphStyle::Heading1, ParagraphStyle::Quote] {
        for alignment in [Alignment::Left, Alignment::Center, Alignment::Right, Alignment::Justify] {
            let doc = Document {
                metadata: Metadata::default(),
                blocks: vec![Block::Paragraph(Paragraph {
                    style,
                    alignment,
                    list: None,
                    runs: vec![Run::plain("Texte")],
                })],
            };
            for format in [FileFormat::Odt, FileFormat::Docx] {
                let back = read_bytes(&write_bytes(&doc, format).unwrap(), format).unwrap();
                assert_eq!(back.blocks, doc.blocks, "{format:?} {style:?} {alignment:?}");
            }
        }
    }
}
