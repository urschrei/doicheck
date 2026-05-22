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

/// Compare metadata against reference text, recording one discrepancy per field
/// that is present in the metadata but does not match the reference.
pub fn compare(reference: &str, meta: &Metadata) -> Vec<Discrepancy> {
    let mut out = Vec::new();

    if let Some(title) = meta.title.as_deref().filter(|t| !t.is_empty())
        && token_coverage(reference, title) < TITLE_THRESHOLD
    {
        out.push(discrepancy(
            Field::Title,
            "(title not found in reference)",
            title.to_string(),
        ));
    }

    if let Some(surname) = meta
        .first_author_surname
        .as_deref()
        .filter(|s| !s.is_empty())
        && token_coverage(reference, surname) < AUTHOR_THRESHOLD
    {
        out.push(discrepancy(
            Field::Author,
            "(first author not found in reference)",
            surname.to_string(),
        ));
    }

    if let Some(year) = meta.year {
        let year = year.to_string();
        if !year_present(reference, &year) {
            out.push(discrepancy(
                Field::Year,
                "(year not found in reference)",
                year,
            ));
        }
    }

    if let Some(container) = meta.container_title.as_deref().filter(|c| !c.is_empty())
        && token_coverage(reference, container) < CONTAINER_THRESHOLD
    {
        out.push(discrepancy(
            Field::Container,
            "(journal/container not found in reference)",
            container.to_string(),
        ));
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
}
