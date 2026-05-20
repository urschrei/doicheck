//! Reading a file, computing its fingerprint, and determining its kind.

use crate::model::FileKind;
use sha2::{Digest, Sha256};
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum IngestError {
    #[error("could not read file: {0}")]
    Io(#[from] std::io::Error),
    #[error("unsupported file type: {0}")]
    UnsupportedKind(String),
}

pub struct Ingested {
    pub bytes: Vec<u8>,
    pub fingerprint: String,
    pub kind: FileKind,
    pub filename: String,
}

/// SHA-256 of the bytes, formatted as `sha256:<hex>`.
pub fn fingerprint(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(digest.len() * 2);
    for b in digest.iter() {
        let _ = write!(hex, "{b:02x}");
    }
    format!("sha256:{hex}")
}

pub fn kind_from_path(path: &Path) -> Result<FileKind, IngestError> {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
    {
        Some(ext) if ext == "pdf" => Ok(FileKind::Pdf),
        Some(ext) if ext == "docx" => Ok(FileKind::Docx),
        other => Err(IngestError::UnsupportedKind(other.unwrap_or_default())),
    }
}

pub fn ingest(path: &Path) -> Result<Ingested, IngestError> {
    let kind = kind_from_path(path)?;
    let bytes = std::fs::read(path)?;
    let fingerprint = fingerprint(&bytes);
    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();
    Ok(Ingested {
        bytes,
        fingerprint,
        kind,
        filename,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn fingerprint_is_stable_and_prefixed() {
        let fp = fingerprint(b"hello");
        assert_eq!(
            fp,
            "sha256:2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    fn kind_detected_case_insensitively() {
        assert_eq!(
            kind_from_path(&PathBuf::from("a.PDF")).unwrap(),
            FileKind::Pdf
        );
        assert_eq!(
            kind_from_path(&PathBuf::from("a.docx")).unwrap(),
            FileKind::Docx
        );
        assert!(kind_from_path(&PathBuf::from("a.txt")).is_err());
    }
}
