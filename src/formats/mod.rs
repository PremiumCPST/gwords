//! File format readers and writers. Pure Rust (zip + quick-xml).
//!
//! Every format module exposes:
//!   `pub fn read(bytes: &[u8]) -> Result<Document, FormatError>`
//!   `pub fn write(doc: &Document) -> Result<Vec<u8>, FormatError>`

use std::path::Path;

use crate::document::Document;

pub mod docx_reader;
pub mod docx_writer;
pub mod odt_reader;
pub mod odt_writer;
pub mod plain;

#[derive(Debug, thiserror::Error)]
pub enum FormatError {
    #[error("Erreur d'entrée/sortie : {0}")]
    Io(#[from] std::io::Error),
    #[error("Archive invalide : {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("XML invalide : {0}")]
    Xml(String),
    #[error("Document invalide : {0}")]
    Invalid(String),
    #[error("Format de fichier non pris en charge : {0}")]
    Unsupported(String),
}

impl From<quick_xml::Error> for FormatError {
    fn from(e: quick_xml::Error) -> Self {
        FormatError::Xml(e.to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileFormat {
    Odt,
    Docx,
    PlainText,
}

impl FileFormat {
    /// Detects the format from the file extension (case-insensitive).
    /// Unknown extensions are treated as plain text.
    pub fn from_path(path: &Path) -> FileFormat {
        match path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref() {
            Some("odt") => FileFormat::Odt,
            Some("docx") => FileFormat::Docx,
            _ => FileFormat::PlainText,
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            FileFormat::Odt => "odt",
            FileFormat::Docx => "docx",
            FileFormat::PlainText => "txt",
        }
    }

    pub fn mime_type(self) -> &'static str {
        match self {
            FileFormat::Odt => "application/vnd.oasis.opendocument.text",
            FileFormat::Docx => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            FileFormat::PlainText => "text/plain",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            FileFormat::Odt => "Document OpenDocument (.odt)",
            FileFormat::Docx => "Document Word (.docx)",
            FileFormat::PlainText => "Texte brut (.txt)",
        }
    }
}

pub fn read_bytes(bytes: &[u8], format: FileFormat) -> Result<Document, FormatError> {
    match format {
        FileFormat::Odt => odt_reader::read(bytes),
        FileFormat::Docx => docx_reader::read(bytes),
        FileFormat::PlainText => plain::read(bytes),
    }
}

pub fn write_bytes(doc: &Document, format: FileFormat) -> Result<Vec<u8>, FormatError> {
    match format {
        FileFormat::Odt => odt_writer::write(doc),
        FileFormat::Docx => docx_writer::write(doc),
        FileFormat::PlainText => plain::write(doc),
    }
}

pub fn load(path: &Path) -> Result<Document, FormatError> {
    let bytes = std::fs::read(path)?;
    read_bytes(&bytes, FileFormat::from_path(path))
}

pub fn save(doc: &Document, path: &Path) -> Result<(), FormatError> {
    let bytes = write_bytes(doc, FileFormat::from_path(path))?;
    std::fs::write(path, bytes)?;
    Ok(())
}
