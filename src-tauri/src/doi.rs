//! DOI extraction from free text and normalisation.

use regex::Regex;
use std::sync::LazyLock;

static DOI_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)10\.\d{1,9}/[-._;()/:a-z0-9]+").unwrap());

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
        let text = "see 10.1/aaa and 10.2/bbb and again 10.1/AAA.";
        assert_eq!(extract_all(text), vec!["10.1/aaa", "10.2/bbb"]);
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
}
