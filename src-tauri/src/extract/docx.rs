//! Plain-text extraction from a DOCX (a zip containing `word/document.xml`).

use quick_xml::events::Event;
use quick_xml::reader::Reader;
use std::io::Read;

#[derive(Debug, thiserror::Error)]
pub enum DocxError {
    #[error("not a valid docx archive: {0}")]
    Zip(String),
    #[error("docx has no word/document.xml")]
    NoDocument,
    #[error("could not read word/document.xml: {0}")]
    Io(String),
}

/// Extract visible paragraph text. Each `<w:t>` run contributes text; each
/// `<w:p>` ends a line.
pub fn extract(bytes: &[u8]) -> Result<String, DocxError> {
    let reader = std::io::Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(reader).map_err(|e| DocxError::Zip(e.to_string()))?;
    let mut xml = String::new();
    archive
        .by_name("word/document.xml")
        .map_err(|_| DocxError::NoDocument)?
        .read_to_string(&mut xml)
        .map_err(|e| DocxError::Io(e.to_string()))?;
    Ok(xml_to_text(&xml))
}

fn xml_to_text(xml: &str) -> String {
    let mut reader = Reader::from_str(xml);
    let mut out = String::new();
    let mut in_text = false;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) if e.local_name().as_ref() == b"t" => in_text = true,
            Ok(Event::End(e)) if e.local_name().as_ref() == b"t" => in_text = false,
            Ok(Event::End(e)) if e.local_name().as_ref() == b"p" => out.push('\n'),
            Ok(Event::Text(e)) if in_text => {
                if let Ok(decoded) = e.decode() {
                    match quick_xml::escape::unescape(&decoded) {
                        Ok(s) => out.push_str(&s),
                        Err(_) => out.push_str(&decoded),
                    }
                }
            }
            // Best-effort: on EOF or any parse error, stop and return the text
            // gathered so far rather than failing the whole extraction.
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_text_from_fixture() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sample.docx");
        let bytes = std::fs::read(path).unwrap();
        let text = extract(&bytes).unwrap();
        assert!(text.contains("References"));
        assert!(text.contains("10.1000/abc"));
    }
}
