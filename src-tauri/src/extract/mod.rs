//! Text extraction dispatch by file kind.

pub mod docx;
pub mod pdf;

use crate::model::FileKind;

#[derive(Debug, thiserror::Error)]
pub enum ExtractError {
    #[error(transparent)]
    Pdf(#[from] pdf::PdfError),
    #[error(transparent)]
    Docx(#[from] docx::DocxError),
}

pub fn extract_text(bytes: &[u8], kind: FileKind) -> Result<String, ExtractError> {
    match kind {
        FileKind::Pdf => Ok(pdf::extract(bytes)?),
        FileKind::Docx => Ok(docx::extract(bytes)?),
    }
}

/// Heuristic: treat near-empty extraction as "no usable text".
pub fn has_usable_text(text: &str) -> bool {
    text.chars().filter(|c| c.is_alphanumeric()).count() >= 20
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_empty_extraction() {
        assert!(!has_usable_text("   \n  "));
        assert!(has_usable_text(
            "This document contains sufficient alphanumeric content."
        ));
    }
}
