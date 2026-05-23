//! Fuzzy comparison of Crossref metadata against the raw reference text.

use crate::model::Discrepancy;
use crate::text::{normalise, token_coverage};

/// Subset of Crossref metadata used for comparison.
#[derive(Debug, Clone, Default)]
pub struct Metadata {
    pub title: Option<String>,
    pub first_author_surname: Option<String>,
    pub year: Option<i32>,
    pub container_title: Option<String>,
}

const TITLE_THRESHOLD: f64 = 0.8;
const CONTAINER_THRESHOLD: f64 = 0.7;
/// The surname must appear in full: every token of it present in the reference.
const AUTHOR_THRESHOLD: f64 = 1.0;

/// A reference field that can disagree with Crossref metadata. The string form
/// is the wire contract carried on `Discrepancy.field` to storage and the UI.
#[derive(Debug, Clone, Copy)]
enum Field {
    Title,
    Author,
    Year,
    Container,
}

impl Field {
    fn as_str(self) -> &'static str {
        match self {
            Field::Title => "title",
            Field::Author => "author",
            Field::Year => "year",
            Field::Container => "container",
        }
    }
}

fn discrepancy(field: Field, reference_value: &str, crossref_value: String) -> Discrepancy {
    Discrepancy {
        field: field.as_str().into(),
        reference_value: reference_value.into(),
        crossref_value,
        dismissed: false,
    }
}

/// Whether `year` appears as a whole token in the reference. Matching on tokens
/// (after stripping a trailing letter suffix) avoids two failure modes of a
/// plain substring search: it does not falsely match a year inside a larger
/// number such as a page or volume (e.g. `32020`), yet it still accepts an
/// author-date suffix such as `2020a`.
fn year_present(reference: &str, year: &str) -> bool {
    normalise(reference)
        .split_whitespace()
        .any(|tok| tok.trim_end_matches(char::is_alphabetic) == year)
}

/// Plausible four-digit year range, wide enough for older reprints yet narrow
/// enough to reject most volume and page numbers.
const YEAR_RANGE: std::ops::RangeInclusive<u32> = 1000..=2100;

/// The year a reference appears to cite, used to contrast with the Crossref year
/// when the two disagree. Called only once the Crossref year is known to be
/// absent, so any year found here necessarily differs from it. A parenthesised
/// author-date year is the most reliable signal, so one inside parentheses wins;
/// otherwise a single unambiguous four-digit token is used. Returns `None` when
/// nothing suitable is found or several distinct years compete, leaving the
/// caller to report the year as simply absent rather than guessing.
fn find_reference_year(reference: &str) -> Option<String> {
    let mut parenthesised: Option<&str> = None;
    let mut bare: Vec<&str> = Vec::new();
    for token in reference.split_whitespace() {
        let in_parens = token.contains('(');
        // Trim surrounding punctuation, then drop a trailing author-date suffix
        // letter (the "a" in "2020a").
        let core = token
            .trim_matches(|c: char| !c.is_ascii_alphanumeric())
            .trim_end_matches(|c: char| c.is_ascii_alphabetic());
        if core.len() == 4 && core.parse::<u32>().is_ok_and(|y| YEAR_RANGE.contains(&y)) {
            if in_parens {
                parenthesised.get_or_insert(core);
            } else {
                bare.push(core);
            }
        }
    }
    if let Some(year) = parenthesised {
        return Some(year.to_string());
    }
    // Only commit to a bare year when the reference cites exactly one distinct
    // four-digit candidate; competing years are ambiguous.
    let first = bare.first()?;
    bare.iter().all(|y| y == first).then(|| first.to_string())
}

/// Compare metadata against reference text, recording one discrepancy per field
/// that is present in the metadata but does not match the reference.
pub fn compare(reference: &str, meta: &Metadata) -> Vec<Discrepancy> {
    let mut out = Vec::new();

    if let Some(title) = meta.title.as_deref().filter(|t| !t.is_empty())
        && token_coverage(reference, title) < TITLE_THRESHOLD
    {
        out.push(discrepancy(Field::Title, "", title.to_string()));
    }

    if let Some(surname) = meta
        .first_author_surname
        .as_deref()
        .filter(|s| !s.is_empty())
        && token_coverage(reference, surname) < AUTHOR_THRESHOLD
    {
        out.push(discrepancy(Field::Author, "", surname.to_string()));
    }

    if let Some(year) = meta.year {
        let year = year.to_string();
        if !year_present(reference, &year) {
            let reference_year = find_reference_year(reference).unwrap_or_default();
            out.push(discrepancy(Field::Year, &reference_year, year));
        }
    }

    if let Some(container) = meta.container_title.as_deref().filter(|c| !c.is_empty())
        && token_coverage(reference, container) < CONTAINER_THRESHOLD
    {
        out.push(discrepancy(Field::Container, "", container.to_string()));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta() -> Metadata {
        Metadata {
            title: Some("A Study of Widgets".into()),
            first_author_surname: Some("Smith".into()),
            year: Some(2020),
            container_title: Some("Journal of Widgets".into()),
        }
    }

    #[test]
    fn matching_reference_has_no_discrepancies() {
        let reference = "Smith J (2020). A Study of Widgets. Journal of Widgets, 12(3).";
        assert!(compare(reference, &meta()).is_empty());
    }

    #[test]
    fn wrong_title_and_year_are_recorded() {
        let reference = "Smith J (1999). A Study of Gadgets and Gizmos elsewhere entirely.";
        let d = compare(reference, &meta());
        let fields: Vec<&str> = d.iter().map(|x| x.field.as_str()).collect();
        assert!(fields.contains(&"title"));
        assert!(fields.contains(&"year"));
    }

    // A year that appears only as a substring of a larger number (e.g. a page or
    // volume) must not count as present.
    #[test]
    fn year_inside_larger_number_is_flagged() {
        let reference = "Smith. A Study of Widgets. Journal of Widgets, 32020.";
        let d = compare(reference, &meta());
        let fields: Vec<&str> = d.iter().map(|x| x.field.as_str()).collect();
        assert_eq!(fields, vec!["year"]);
    }

    // An author-date suffix on the year (2020a) must still match the year 2020.
    #[test]
    fn year_with_letter_suffix_matches() {
        let reference = "Smith. A Study of Widgets. Journal of Widgets, 2020a.";
        assert!(compare(reference, &meta()).is_empty());
    }

    fn year_discrepancy(reference: &str) -> Discrepancy {
        compare(reference, &meta())
            .into_iter()
            .find(|d| d.field == "year")
            .expect("expected a year discrepancy")
    }

    // A wrong but present year is carried as the reference value so the UI can
    // show it against the Crossref year. The parenthesised author-date year is
    // preferred over any other four-digit token.
    #[test]
    fn wrong_year_records_the_cited_year() {
        let d = year_discrepancy("Smith J (2011). A Study of Widgets. Journal of Widgets, 12(3).");
        assert_eq!(d.reference_value, "2011");
        assert_eq!(d.crossref_value, "2020");
    }

    // With no year at all in the reference, there is nothing to contrast and the
    // reference value stays empty.
    #[test]
    fn absent_year_has_empty_reference_value() {
        let d = year_discrepancy("Smith J. A Study of Widgets. Journal of Widgets, 12(3).");
        assert!(d.reference_value.is_empty());
    }

    // Two competing four-digit years (e.g. a publication and an access date) are
    // ambiguous, so no value is guessed.
    #[test]
    fn ambiguous_years_are_not_guessed() {
        let d = year_discrepancy("Smith J. A Study of Widgets. 1998. Accessed 2007.");
        assert!(d.reference_value.is_empty());
    }

    // A four-digit number that is only ever part of a larger token is not a year.
    #[test]
    fn page_or_volume_number_is_not_treated_as_a_year() {
        let d = year_discrepancy("Smith. A Study of Widgets. Journal of Widgets, 32020.");
        assert!(d.reference_value.is_empty());
    }

    // Fields whose value cannot be located in the reference carry no reference
    // value at all.
    #[test]
    fn absent_non_year_fields_have_empty_reference_value() {
        let reference = "Jones Q (2020). Something unrelated entirely about gizmos.";
        let d = compare(reference, &meta());
        for field in ["title", "author", "container"] {
            let found = d
                .iter()
                .find(|x| x.field == field)
                .unwrap_or_else(|| panic!("expected a {field} discrepancy"));
            assert!(found.reference_value.is_empty(), "{field}");
        }
    }
}
