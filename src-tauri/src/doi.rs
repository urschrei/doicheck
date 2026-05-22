//! DOI extraction from free text and normalisation.

use regex::Regex;
use std::collections::HashSet;
use std::sync::LazyLock;

static DOI_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)10\.\d{4,9}/[-._;()/:a-z0-9]+").unwrap());

/// A DOI in normalised form. The only constructor runs [`normalise`], so every
/// `Doi` value is a lower-cased, canonical key. Use it at API boundaries (e.g.
/// the cache) to make the normalisation invariant a compile-time guarantee
/// rather than a caller obligation.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Doi(String);

impl Doi {
    /// Normalise `raw` into a `Doi`.
    pub fn new(raw: &str) -> Self {
        Doi(normalise(raw))
    }

    /// The normalised DOI string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

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
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for m in DOI_RE.find_iter(text) {
        let doi = normalise(m.as_str());
        if seen.insert(doi.clone()) {
            out.push(doi);
        }
    }
    out
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
    let mut seen: HashSet<String> = HashSet::new();
    let mut prev_end = 0usize;
    for m in DOI_RE.find_iter(text) {
        let doi = normalise(m.as_str());
        if seen.contains(&doi) {
            prev_end = m.end();
            continue;
        }
        let lower_bound = m.start().saturating_sub(WINDOW).max(prev_end);
        let start = snap_char_boundary(text, lower_bound);
        let window = trim_to_last_entry_start(&text[start..m.end()]);
        let context = window.split_whitespace().collect::<Vec<_>>().join(" ");
        out.push((doi.clone(), context));
        seen.insert(doi);
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

/// Trim a context window to start at the last line that begins a reference entry,
/// so the window holds only the reference that owns the DOI. If no entry start is
/// found, return the whole window unchanged.
fn trim_to_last_entry_start(window: &str) -> &str {
    let mut best = 0usize;
    let mut found = false;
    let mut offset = 0usize;
    for line in window.split_inclusive('\n') {
        if crate::biblio::is_entry_start(line) {
            best = offset;
            found = true;
        }
        offset += line.len();
    }
    if found { &window[best..] } else { window }
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

    #[test]
    fn doi_newtype_normalises_on_construction() {
        assert_eq!(
            Doi::new("https://doi.org/10.1000/XYZ.").as_str(),
            "10.1000/xyz"
        );
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

    // A window must not include a prior reference that happened to lack a DOI.
    #[test]
    fn context_excludes_prior_doiless_reference() {
        let text = "Atkinson, R. & Easthope, H. (2008) 'Creative Class'. https://www.jstor.org/stable/23289786\n\
Black, J. (2026) 'Towards Net-Zero'. https://doi.org/10.7916/qbtt-xa42\n";
        let v = extract_with_context(text);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].0, "10.7916/qbtt-xa42");
        assert!(v[0].1.contains("Black"));
        assert!(!v[0].1.contains("Atkinson"));
    }
}
