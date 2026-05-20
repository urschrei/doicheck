//! Locating the bibliography in extracted text and splitting it into entries.

use crate::model::ReferenceEntry;
use regex::Regex;
use std::sync::LazyLock;

static HEADING_RE: LazyLock<Regex> = LazyLock::new(|| {
    // Optional leading section number or Roman numeral, then the keyword as
    // effectively the whole line. Trailing dotted leaders/page numbers (a
    // table-of-contents entry) prevent a match.
    Regex::new(
        r"(?im)^\s*(?:\d+\.?\s+|[ivxlcdm]+\.?\s+)?(references|bibliography|works cited|literature cited)\s*$",
    )
    .unwrap()
});

// A numbered marker at the start of an entry, e.g. "[12]" or "12." or "12)".
static NUMBER_MARKER_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^\s*(?:\[\d+\]|\d+[.)])\s+").unwrap());

static YEAR_PAREN_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\(\d{4}[a-z]?\)").unwrap());

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

/// A line begins a new entry if it carries a numbered marker, or it looks like
/// an author-date opening: starts with an uppercase letter and has a
/// parenthesised year near the start.
fn is_entry_start(line: &str) -> bool {
    if NUMBER_MARKER_RE.is_match(line) {
        return true;
    }
    let trimmed = line.trim_start();
    let begins_upper = trimmed.chars().next().is_some_and(|c| c.is_uppercase());
    begins_upper && YEAR_PAREN_RE.find(trimmed).is_some_and(|m| m.start() <= 80)
}

/// Split a bibliography section into entries by detecting entry starts and
/// joining wrapped continuation lines. Falls back to blank-line paragraphs if
/// no entry starts are found.
pub fn split_entries(section: &str) -> Vec<String> {
    let mut entries: Vec<String> = Vec::new();
    let mut current: Option<String> = None;
    for line in section.lines() {
        if is_entry_start(line) {
            if let Some(buf) = current.take() {
                let cleaned = collapse_ws(&buf);
                if !cleaned.is_empty() {
                    entries.push(cleaned);
                }
            }
            current = Some(line.to_string());
        } else if let Some(buf) = current.as_mut() {
            buf.push(' ');
            buf.push_str(line);
        }
    }
    if let Some(buf) = current {
        let cleaned = collapse_ws(&buf);
        if !cleaned.is_empty() {
            entries.push(cleaned);
        }
    }

    if entries.is_empty() {
        // No detectable entry starts: fall back to blank-line paragraphs.
        return section
            .split("\n\n")
            .map(collapse_ws)
            .filter(|s| !s.is_empty())
            .collect();
    }
    entries
}

fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Detect and segment the bibliography from full document text. If no heading is
/// found, fall back to DOI-anchored windows so comparison still has real text.
pub fn detect(text: &str) -> Bibliography {
    if let Some(section) = section_after_heading(text) {
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
    } else {
        let entries = crate::doi::extract_with_context(text)
            .into_iter()
            .enumerate()
            .map(|(i, (doi, raw_text))| ReferenceEntry {
                ordinal: i + 1,
                raw_text,
                doi: Some(doi),
            })
            .collect();
        Bibliography {
            detected: false,
            entries,
        }
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
    fn no_heading_falls_back_to_doi_windows() {
        let bib = detect("Just a body with 10.1000/xyz inline and no heading line.");
        assert!(!bib.detected);
        assert_eq!(bib.entries.len(), 1);
        assert_eq!(bib.entries[0].doi.as_deref(), Some("10.1000/xyz"));
        // The window carries surrounding text, not just the bare DOI.
        assert!(bib.entries[0].raw_text.contains("body"));
    }

    #[test]
    fn heading_allows_section_number_but_not_toc() {
        // Real heading with a section number on its own line.
        let text = "body\n 6. References  \nAdams, D. (2012). Title. 10.4324/9780203857007";
        assert!(section_after_heading(text).is_some());
        // A table-of-contents line (dotted leaders + page number) must NOT match.
        let toc = "6. References .......................................... 13\nmore body";
        assert!(section_after_heading(toc).is_none());
    }

    #[test]
    fn heading_still_matches_plain_keywords() {
        assert!(section_after_heading("x\nReferences\n[1] A 10.1000/a").is_some());
        assert!(section_after_heading("x\nBibliography\nA 10.1000/a").is_some());
    }

    // Models how pdf-extract renders an author-date reference list: a numbered
    // heading, hanging-indent wrapping, and blank lines within and between
    // entries. The third entry has no DOI (a handle.net link).
    const SAMPLE: &str = "Some preamble paragraph.\n \n 6. References  \n \n\
Adams, D., & Watkins, C. (2012). Urban Planning and the Development Process. Routledge. \n \n\
https://doi.org/10.4324/9780203857007 \n \n\
Arnstein, S. R. (1969). A Ladder of Citizen Participation. Journal of the American Institute of \n \n\
Planners, 35(4), 216-224. https://doi.org/10.1080/01944366908977225 \n \n\
Malfer, B. (2025). Smart Cities in the European Union. Handle.Net. \n \n\
https://hdl.handle.net/20.500.12608/83965 \n";

    #[test]
    fn segments_wrapped_author_date_entries() {
        let bib = detect(SAMPLE);
        assert!(bib.detected);
        assert_eq!(bib.entries.len(), 3);
        assert_eq!(bib.entries[0].doi.as_deref(), Some("10.4324/9780203857007"));
        // The second entry's wrapped continuation line must be joined in.
        assert!(bib.entries[1].raw_text.contains("Planners"));
        assert_eq!(
            bib.entries[1].doi.as_deref(),
            Some("10.1080/01944366908977225")
        );
        // The handle.net entry has no DOI.
        assert_eq!(bib.entries[2].doi, None);
        assert!(bib.entries[2].raw_text.contains("Malfer"));
    }
}
