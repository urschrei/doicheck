//! Fuzzy comparison of Crossref metadata against the raw reference text.

use crate::model::Discrepancy;
use crate::text::token_coverage;

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

/// Compare metadata against reference text, recording one discrepancy per field
/// that is present in the metadata but does not match the reference.
pub fn compare(reference: &str, meta: &Metadata) -> Vec<Discrepancy> {
    let mut out = Vec::new();

    if let Some(title) = meta.title.as_deref().filter(|t| !t.is_empty()) {
        if token_coverage(reference, title) < TITLE_THRESHOLD {
            out.push(Discrepancy {
                field: "title".into(),
                reference_value: "(title not found in reference)".into(),
                crossref_value: title.to_string(),
            });
        }
    }

    if let Some(surname) = meta
        .first_author_surname
        .as_deref()
        .filter(|s| !s.is_empty())
    {
        if token_coverage(reference, surname) < 1.0 {
            out.push(Discrepancy {
                field: "author".into(),
                reference_value: "(first author not found in reference)".into(),
                crossref_value: surname.to_string(),
            });
        }
    }

    if let Some(year) = meta.year {
        if !crate::text::normalise(reference).contains(&year.to_string()) {
            out.push(Discrepancy {
                field: "year".into(),
                reference_value: "(year not found in reference)".into(),
                crossref_value: year.to_string(),
            });
        }
    }

    if let Some(container) = meta.container_title.as_deref().filter(|c| !c.is_empty()) {
        if token_coverage(reference, container) < CONTAINER_THRESHOLD {
            out.push(Discrepancy {
                field: "container".into(),
                reference_value: "(journal/container not found in reference)".into(),
                crossref_value: container.to_string(),
            });
        }
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
}
