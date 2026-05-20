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
}
