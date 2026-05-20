//! Plain-text extraction from a PDF using the `pdf-extract` crate.

#[derive(Debug, thiserror::Error)]
pub enum PdfError {
    #[error("could not extract text from pdf: {0}")]
    Extract(String),
}

pub fn extract(bytes: &[u8]) -> Result<String, PdfError> {
    pdf_extract::extract_text_from_mem(bytes).map_err(|e| PdfError::Extract(e.to_string()))
}
