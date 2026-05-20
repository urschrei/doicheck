//! Locating the bibliography in extracted text and splitting it into entries.

use crate::model::ReferenceEntry;
use regex::Regex;
use std::sync::LazyLock;

static HEADING_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?im)^\s*(references|bibliography|works cited|literature cited)\s*$").unwrap()
});

// A numbered marker at the start of an entry, e.g. "[12]" or "12." or "12)".
static NUMBER_MARKER_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^\s*(?:\[\d+\]|\d+[.)])\s+").unwrap());

#[derive(Debug, PartialEq, Eq)]
pub struct Bibliography {
    pub detected: bool,
    pub entries: Vec<ReferenceEntry>,
}

/// Find the bibliography section (the last matching heading) and return the
/// text after it. Returns `None` if no heading is found.
pub fn section_after_heading(text: &str) -> Option<&str> {
    let last = HEADING_RE.find_iter(text).last()?;
    Some(&text[last.end()..])
}

/// Split a bibliography section into entries. Prefers numbered markers; falls
/// back to splitting on blank lines.
pub fn split_entries(section: &str) -> Vec<String> {
    let marker_count = NUMBER_MARKER_RE.find_iter(section).count();
    if marker_count >= 2 {
        return split_on_markers(section);
    }
    // Blank-line separated paragraphs.
    section
        .split("\n\n")
        .map(collapse_ws)
        .filter(|s| !s.is_empty())
        .collect()
}

fn split_on_markers(section: &str) -> Vec<String> {
    let mut starts: Vec<usize> = NUMBER_MARKER_RE
        .find_iter(section)
        .map(|m| m.start())
        .collect();
    starts.push(section.len());
    let mut out = Vec::new();
    for w in starts.windows(2) {
        let chunk = &section[w[0]..w[1]];
        let cleaned = collapse_ws(&NUMBER_MARKER_RE.replace(chunk, ""));
        if !cleaned.is_empty() {
            out.push(cleaned);
        }
    }
    out
}

fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Detect and segment the bibliography from full document text.
pub fn detect(text: &str) -> Bibliography {
    match section_after_heading(text) {
        Some(section) => {
            let entries = split_entries(section)
                .into_iter()
                .enumerate()
                .map(|(i, raw_text)| ReferenceEntry {
                    ordinal: i + 1,
                    doi: crate::doi::first_in(&raw_text),
                    raw_text,
                })
                .collect();
            Bibliography {
                detected: true,
                entries,
            }
        }
        None => Bibliography {
            detected: false,
            entries: Vec::new(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_section_after_last_heading() {
        let text = "Intro mentions references casually.\nReferences\n[1] A\n[2] B";
        let section = section_after_heading(text).unwrap();
        assert!(section.contains("[1] A"));
        assert!(!section.contains("Intro"));
    }

    #[test]
    fn splits_numbered_entries_and_finds_dois() {
        let section = "\n[1] Smith J. Title. 10.1000/aaa\n[2] Jones K. Other. 10.2000/bbb\n";
        let bib = detect(&format!("References{section}"));
        assert!(bib.detected);
        assert_eq!(bib.entries.len(), 2);
        assert_eq!(bib.entries[0].ordinal, 1);
        assert_eq!(bib.entries[0].doi.as_deref(), Some("10.1000/aaa"));
        assert_eq!(bib.entries[1].doi.as_deref(), Some("10.2000/bbb"));
    }

    #[test]
    fn undetected_when_no_heading() {
        let bib = detect("Just a body with 10.1000/xyz and no heading line.");
        assert!(!bib.detected);
        assert!(bib.entries.is_empty());
    }
}
