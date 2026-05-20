//! DOI extraction from free text and normalisation.

use regex::Regex;
use std::sync::LazyLock;

static DOI_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)10\.\d{4,9}/[-._;()/:a-z0-9]+").unwrap());

/// Normalise a DOI: drop a URL or `doi:` prefix, lowercase, strip trailing
/// punctuation that commonly clings to DOIs in reference lists.
pub fn normalise(raw: &str) -> String {
    let s = raw.trim();
    let s = s
        .strip_prefix("https://doi.org/")
        .or_else(|| s.strip_prefix("http://doi.org/"))
        .or_else(|| s.strip_prefix("https://dx.doi.org/"))
        .or_else(|| s.strip_prefix("doi:"))
        .or_else(|| s.strip_prefix("DOI:"))
        .unwrap_or(s);
    s.trim_end_matches(['.', ',', ';', ')', ']', '>', '"', '\''])
        .to_lowercase()
}

/// Extract all DOIs from text, normalised and de-duplicated, order preserved.
pub fn extract_all(text: &str) -> Vec<String> {
    let mut seen = Vec::new();
    for m in DOI_RE.find_iter(text) {
        let doi = normalise(m.as_str());
        if !seen.contains(&doi) {
            seen.push(doi);
        }
    }
    seen
}

/// The first DOI in a single reference, if any.
pub fn first_in(text: &str) -> Option<String> {
    DOI_RE.find(text).map(|m| normalise(m.as_str()))
}

/// For each DOI in the text, return the DOI plus the text immediately preceding
/// it (back to the previous DOI, capped at a window), de-wrapped. Used as a
/// fallback when no bibliography heading is detected, so comparison still has
/// real reference text to work with. De-duplicates by DOI.
pub fn extract_with_context(text: &str) -> Vec<(String, String)> {
    const WINDOW: usize = 600;
    let mut out: Vec<(String, String)> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    let mut prev_end = 0usize;
    for m in DOI_RE.find_iter(text) {
        let doi = normalise(m.as_str());
        if seen.contains(&doi) {
            prev_end = m.end();
            continue;
        }
        let lower_bound = m.start().saturating_sub(WINDOW).max(prev_end);
        let start = snap_char_boundary(text, lower_bound);
        let context = text[start..m.end()]
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        out.push((doi.clone(), context));
        seen.push(doi);
        prev_end = m.end();
    }
    out
}

fn snap_char_boundary(s: &str, mut i: usize) -> usize {
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalises_url_and_trailing_punctuation() {
        assert_eq!(normalise("https://doi.org/10.1000/XYZ."), "10.1000/xyz");
        assert_eq!(normalise("doi:10.1000/Abc),"), "10.1000/abc");
    }

    #[test]
    fn extracts_and_dedupes_in_order() {
        let text = "see 10.1000/aaa and 10.2000/bbb and again 10.1000/AAA.";
        assert_eq!(extract_all(text), vec!["10.1000/aaa", "10.2000/bbb"]);
    }

    #[test]
    fn first_in_finds_none_when_absent() {
        assert_eq!(first_in("no identifier here"), None);
    }

    // An already-normalised DOI must be a fixed point of `normalise`.
    #[test]
    fn normalise_is_idempotent() {
        for raw in ["10.1000/xyz", "10.5555/a.b-c_d"] {
            let once = normalise(raw);
            assert_eq!(normalise(&once), once);
        }
    }

    #[test]
    fn extract_with_context_windows_each_doi() {
        let text = "Smith, J. (2020). A Study of Widgets. Journal. https://doi.org/10.1000/abc \
                    and later Jones (2019). Other Work. 10.2000/def";
        let v = extract_with_context(text);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].0, "10.1000/abc");
        assert!(v[0].1.contains("A Study of Widgets"));
        assert_eq!(v[1].0, "10.2000/def");
        assert!(v[1].1.contains("Other Work"));
        // The second window starts after the first DOI, so it must not
        // contain the first entry's title.
        assert!(!v[1].1.contains("Widgets"));
    }
}
