//! Shared text normalisation used by comparison and search.

use deunicode::deunicode;

/// Lowercase, transliterate diacritics, reduce to alphanumeric tokens.
pub fn normalise(s: &str) -> String {
    let lower = deunicode(s).to_lowercase();
    lower
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn tokens(s: &str) -> Vec<String> {
    normalise(s)
        .split_whitespace()
        .map(|t| t.to_string())
        .collect()
}

/// Fraction (0.0-1.0) of `needle` tokens present in `haystack` tokens.
pub fn token_coverage(haystack: &str, needle: &str) -> f64 {
    let hay: std::collections::HashSet<String> = tokens(haystack).into_iter().collect();
    let need = tokens(needle);
    if need.is_empty() {
        return 0.0;
    }
    let found = need.iter().filter(|t| hay.contains(*t)).count();
    found as f64 / need.len() as f64
}

/// Whether a reference string has enough non-identifier text to compare against
/// Crossref metadata. Strips URL/DOI tokens, then requires a minimum count of
/// alphanumeric characters. Prevents false discrepancies for entries whose only
/// content is a DOI (e.g. a sparse fallback window).
pub fn is_comparable(reference: &str) -> bool {
    let without_ids: String = reference
        .split_whitespace()
        .filter(|t| {
            let l = t.to_ascii_lowercase();
            !l.starts_with("http") && !l.starts_with("10.") && !l.contains("doi.org")
        })
        .collect::<Vec<_>>()
        .join(" ");
    without_ids.chars().filter(|c| c.is_alphanumeric()).count() >= 15
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalise_strips_diacritics_and_punctuation() {
        assert_eq!(normalise("Crème brûlée, 2020!"), "creme brulee 2020");
    }

    #[test]
    fn token_coverage_is_fraction_present() {
        let haystack = "smith j a study of widgets journal 2020";
        assert_eq!(token_coverage(haystack, "a study of widgets"), 1.0);
        assert!((token_coverage(haystack, "a study of gadgets") - 0.75).abs() < 1e-9);
    }

    #[test]
    fn is_comparable_requires_real_text() {
        assert!(!is_comparable("10.1000/abc"));
        assert!(!is_comparable("https://doi.org/10.1000/abc"));
        assert!(is_comparable(
            "Smith, J. (2020). A Study of Widgets. Journal of Widgets."
        ));
    }
}
