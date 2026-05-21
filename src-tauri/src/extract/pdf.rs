//! Plain-text extraction from a PDF.
//!
//! Primary extractor is PDFium (via `pdfium-render`), which handles subset-font
//! and glyph-spacing cases that the pure-Rust `pdf-extract` mishandles (it can
//! emit spurious spaces inside words, breaking DOI detection). If the PDFium
//! library cannot be located or fails, we fall back to `pdf-extract`.

use std::path::PathBuf;
use std::sync::OnceLock;

use pdfium_render::prelude::*;

#[derive(Debug, thiserror::Error)]
pub enum PdfError {
    #[error("could not extract text from pdf: {0}")]
    Extract(String),
}

type Bindings = Box<dyn PdfiumLibraryBindings>;

/// Directory holding the bundled PDFium library, set once at startup from the
/// app's resource directory. Optional: binding also tries `PDFIUM_LIB_DIR`, the
/// executable's directory, and the system library.
static PDFIUM_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Configure where to find the PDFium library (the app resource directory).
pub fn set_library_dir(dir: PathBuf) {
    let _ = PDFIUM_DIR.set(dir);
}

pub fn extract(bytes: &[u8]) -> Result<String, PdfError> {
    match extract_with_pdfium(bytes) {
        Ok(text) => Ok(text),
        Err(e) => {
            eprintln!("PDFium extraction unavailable or failed ({e}); using pdf-extract fallback");
            pdf_extract::extract_text_from_mem(bytes).map_err(|e| PdfError::Extract(e.to_string()))
        }
    }
}

fn extract_with_pdfium(bytes: &[u8]) -> Result<String, String> {
    let pdfium = Pdfium::new(bind_pdfium()?);
    let document = pdfium
        .load_pdf_from_byte_slice(bytes, None)
        .map_err(|e| e.to_string())?;
    let mut out = String::new();
    for page in document.pages().iter() {
        let text = page.text().map_err(|e| e.to_string())?;
        out.push_str(&text.all());
        out.push('\n');
    }
    Ok(out)
}

fn bind_pdfium() -> Result<Bindings, String> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(dir) = PDFIUM_DIR.get() {
        dirs.push(dir.clone());
    }
    if let Ok(dir) = std::env::var("PDFIUM_LIB_DIR") {
        dirs.push(PathBuf::from(dir));
    }
    if let Some(parent) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(PathBuf::from))
    {
        dirs.push(parent);
    }
    for dir in dirs {
        let name = Pdfium::pdfium_platform_library_name_at_path(&dir);
        if let Ok(bindings) = Pdfium::bind_to_library(name) {
            return Ok(bindings);
        }
    }
    Pdfium::bind_to_system_library().map_err(|e| e.to_string())
}
