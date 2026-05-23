//! Locating the bibliography in extracted text and splitting it into entries.

use crate::model::ReferenceEntry;
use regex::Regex;
use std::sync::LazyLock;

static HEADING_RE: LazyLock<Regex> = LazyLock::new(|| {
    // Optional leading section number or Roman numeral, then the keyword as
    // effectively the whole line. Trailing dotted leaders/page numbers (a
    // table-of-contents entry) prevent a match.
    Regex::new(
        r"(?im)^\s*(?:\d+\.?\s+|[ivxlcdm]+\.?\s+)?(references|reference list|bibliography|works cited|literature cited|resources|sources)\s*[:.]?\s*$",
    )
    .unwrap()
});

// Headings that mark the end of the bibliography (the start of a later section),
// so trailing matter is not treated as references. The appendix matcher stays
// loose (a title may follow on the same line); the remaining terminators must be
// the whole line, so a citation that merely begins with the word (e.g.
// "Declaration of Helsinki (2013) ...") is not mistaken for a heading.
static END_HEADING_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?im)^\s*(?:appendix|appendices)\b|^\s*(?:declarations?|statement\s+of\s+\w+(?:\s+\w+)*|acknowledge?ments?|about\s+the\s+authors?|author\s+biograph\w+|biograph(?:y|ies)|biographical\s+notes?)\s*[:.]?\s*$",
    )
    .unwrap()
});

// A numbered marker at the start of an entry, e.g. "[12]" or "12." or "12)".
// The bare form is capped at three digits so a four-digit publication year that
// wraps to the start of a line (e.g. "2024. Title…") is not mistaken for a list
// marker and used as a spurious entry boundary.
static NUMBER_MARKER_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^\s*(?:\[\d+\]|\d{1,3}[.)])\s+").unwrap());

// An author-date opener whose year is *unparenthesised*, as produced by
// EndNote/Word Harvard styles ("SURNAME, A. 2018. Title" or "ORG 2016. Title").
// The year often wraps to the next line, so the author list at the start of the
// line is the signal, not the year. Two shapes: a surname (one or more words,
// possibly all-caps) followed by ", <Initial>."; or an all-caps organisation
// author followed directly by a bare year.
static AUTHOR_START_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:\p{Lu}[\p{L}'’.\- ]*,\s*\p{Lu}\.|\p{Lu}[\p{Lu}'’.\- ]*\s+(?:19|20)\d{2}[.,])")
        .unwrap()
});

// A parenthesised publication year, e.g. "(2025)" or "(2018a)". Constrained to
// 1900-2099 so a journal article/issue number like "Land, 14 (1225)" is not
// mistaken for a year (which would wrongly split a reference mid-entry).
static YEAR_PAREN_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\((?:19|20)\d{2}[a-z]?\)").unwrap());

// An MLA/Chicago opener whose title is quoted: the author block ends in a full
// stop and is followed (near the start of the line) by an opening quote. This
// catches "Surname, Given Names. "Title."" and corporate authors ("IMD. "..."")
// that carry no parenthesised year and spell given names out in full, so neither
// the parenthesised-year branch nor [`AUTHOR_START_RE`] applies. A wrapped
// continuation carries the *closing* quote (U+201D), which is excluded here.
static QUOTED_TITLE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new("^\\p{Lu}.{0,78}?\\.\\s+[\"\u{201c}]").unwrap());

// An MLA/Chicago opener whose title is *not* quoted (a book or report): a
// surname, a comma, one or more capitalised given-name tokens ending in a full
// stop, then a capitalised title word. The given tokens are space-separated and
// cannot cross a comma, so a wrapped journal/place line such as "World Planning
// Congress, Toronto, ON, Canada, 2024." is not mistaken for an opener.
static AUTHOR_TITLE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\p{Lu}[\p{L}'’\- ]*,\s+(?:\p{Lu}[\p{L}'’.\-]*\s+)*\p{Lu}[\p{L}'’.\-]*\.\s+\p{Lu}")
        .unwrap()
});

#[derive(Debug, PartialEq, Eq)]
pub struct Bibliography {
    pub detected: bool,
    pub entries: Vec<ReferenceEntry>,
}

/// Find the bibliography section (the last matching heading) and return the
/// text after it. Returns `None` if no heading is found.
pub fn section_after_heading(text: &str) -> Option<&str> {
    let last = HEADING_RE.find_iter(text).last()?;
    let section = &text[last.end()..];
    // Stop at a later top-level section (e.g. an appendix) so its content is not
    // mistaken for references.
    match END_HEADING_RE.find(section) {
        Some(end) => Some(&section[..end.start()]),
        None => Some(section),
    }
}

/// A line begins a new entry if it carries a numbered marker, or it looks like
/// an author-date opening (an uppercase start with a parenthesised year near the
/// start, or an author list whose year is unparenthesised; see
/// [`AUTHOR_START_RE`]), or an MLA/Chicago author-title opening (see
/// [`QUOTED_TITLE_RE`] and [`AUTHOR_TITLE_RE`]).
pub(crate) fn is_entry_start(line: &str) -> bool {
    if NUMBER_MARKER_RE.is_match(line) {
        return true;
    }
    let trimmed = line.trim_start();
    let begins_upper = trimmed.chars().next().is_some_and(|c| c.is_uppercase());
    // An author-date opener with a parenthesised year near the start. The text
    // before the year must be author-like (carry no digits): a wrapped journal
    // line such as "Top. 214, 481–518 (2012)." has a page range before the year
    // and is a continuation, not a new entry.
    if begins_upper
        && YEAR_PAREN_RE.find(trimmed).is_some_and(|m| {
            m.start() <= 80 && !trimmed[..m.start()].bytes().any(|b| b.is_ascii_digit())
        })
    {
        return true;
    }
    if AUTHOR_START_RE.is_match(trimmed) {
        return true;
    }
    // MLA/Chicago openers, which carry no parenthesised year and spell given names
    // out in full: a quoted title after the author block, or (for an unquoted book
    // or report) "Surname, Given Names. Capitalised Title".
    QUOTED_TITLE_RE.is_match(trimmed) || AUTHOR_TITLE_RE.is_match(trimmed)
}

/// Split a bibliography section into entries by detecting entry starts and
/// joining wrapped continuation lines. Falls back to blank-line paragraphs if
/// no entry starts are found.
pub fn split_entries(section: &str) -> Vec<String> {
    let mut entries: Vec<String> = Vec::new();
    let mut current: Option<String> = None;
    // True when the previous line ended mid-author-list (a trailing "," or "&"),
    // so an author-shaped line that follows is a wrapped continuation of the same
    // entry's author list, not the start of a new entry.
    let mut authors_continue = false;
    for line in section.lines() {
        if is_entry_start(line) && !authors_continue {
            if let Some(buf) = current.take() {
                let cleaned = collapse_ws(&buf);
                if !cleaned.is_empty() {
                    entries.push(cleaned);
                }
            }
            current = Some(line.to_string());
        } else if let Some(buf) = current.as_mut() {
            if continues_url(buf, line) {
                let kept = buf.trim_end().len();
                buf.truncate(kept);
                buf.push_str(line.trim_start());
            } else {
                buf.push(' ');
                buf.push_str(line);
            }
        }
        let end = line.trim_end();
        authors_continue = end.ends_with(',') || end.ends_with('&');
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

/// Whether a wrapped continuation line continues a URL from the previous line,
/// so it should be glued on without an intervening space. PDF line wraps split
/// long URLs across lines; joining with a space would break the link.
fn continues_url(buf: &str, next: &str) -> bool {
    if !buf
        .split_whitespace()
        .next_back()
        .is_some_and(|t| t.contains("://"))
    {
        return false;
    }
    // Glue only when the continuation begins like a URL fragment (a lowercase
    // letter, digit, or path/query character) rather than a capitalised word or
    // a bracketed access note such as "(Accessed ...)".
    next.trim_start()
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "/?#&=%._~-+".contains(c))
}

/// Split a line into a (lowercased prefix, page-number) candidate for a running
/// header. The prefix may be empty (a bare page number). Returns `None` when the
/// line is not header-shaped: the trailing token must be a 1-3 digit integer (a
/// page number, not a year), and the prefix must be short (a header, not a
/// reference line).
fn running_head_parts(line: &str) -> Option<(String, &str)> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let (prefix, num) = match trimmed.rsplit_once(char::is_whitespace) {
        Some((p, n)) => (p.trim(), n),
        None => ("", trimmed),
    };
    if num.len() > 3 || num.is_empty() || !num.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    if prefix.chars().count() > 30 {
        return None;
    }
    Some((prefix.to_lowercase(), num))
}

/// Remove running page headers/footers and bare page numbers. A header-shaped
/// line ("<prefix> <page-number>") is dropped when its prefix recurs on at least
/// three lines with differing numbers; a constant boilerplate line (e.g. an
/// author name or ID printed in the footer) is dropped when its exact text
/// recurs at least three times. Section headings are never dropped.
fn strip_running_heads(text: &str) -> String {
    use std::collections::{HashMap, HashSet};
    // Repeating "<prefix> <page-number>" headers, grouped by prefix.
    let mut groups: HashMap<String, HashSet<&str>> = HashMap::new();
    // Verbatim line frequencies, to catch constant footer/header text that is not
    // page-number shaped (e.g. an author name or a student/ID number).
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for line in text.lines() {
        if let Some((prefix, num)) = running_head_parts(line) {
            groups.entry(prefix).or_default().insert(num);
        }
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            *counts.entry(trimmed).or_default() += 1;
        }
    }
    let strip: HashSet<String> = groups
        .into_iter()
        .filter(|(_, nums)| nums.len() >= 3)
        .map(|(prefix, _)| prefix)
        .collect();
    // A header/footer line is short; the cap avoids dropping a (long) reference
    // even in the unlikely event it is repeated.
    const MAX_BOILERPLATE_CHARS: usize = 50;
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        let trimmed = line.trim();
        let is_header = running_head_parts(line).is_some_and(|(prefix, _)| strip.contains(&prefix));
        let is_boilerplate = !trimmed.is_empty()
            && trimmed.chars().count() <= MAX_BOILERPLATE_CHARS
            && counts.get(trimmed).is_some_and(|&n| n >= 3)
            && !HEADING_RE.is_match(trimmed);
        if !is_header && !is_boilerplate {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// Detect and segment the bibliography from full document text. If no heading is
/// found, fall back to DOI-anchored windows so comparison still has real text.
pub fn detect(text: &str) -> Bibliography {
    let cleaned = strip_running_heads(text);
    if let Some(section) = section_after_heading(&cleaned) {
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
        let entries = crate::doi::extract_with_context(&cleaned)
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

    // A journal article number in parentheses ("Land, 14 (1225)") must not be
    // mistaken for a publication year and split the reference (regression).
    #[test]
    fn issue_number_in_parens_does_not_split_entry() {
        let section = "References\n\
Sun, X. et al. (2025) \"Are we satisfied with the achievements of new eco-city construction in \n\
  China? A case study of the Sino-Singapore Tianjin eco-city\", Land, 14 (1225), pp. \n\
  1-22. Available at: https://doi.org/10.3390/land14061225 \n\
 \n\
Tan, Y. (2026) Tigers and flies. Available at: https://example.com/x \n";
        let bib = detect(section);
        assert!(bib.detected);
        let sun = bib
            .entries
            .iter()
            .find(|e| e.doi.as_deref() == Some("10.3390/land14061225"))
            .expect("the Sun reference should be a single entry carrying its DOI");
        assert!(sun.raw_text.contains("Sun"));
        assert!(sun.raw_text.contains("2025"));
    }

    // Harvard/EndNote author-date with ALL-CAPS surnames and UNPARENTHESISED
    // years (the year often wraps to its own line). Regression for a real term
    // paper whose reference list collapsed into a single entry.
    const CAPS_AUTHOR_DATE: &str = "References\n\
CARDULLO, P. & KITCHIN, R. 2019. Being a 'citizen' in the smart city: Up and down the \n\
scaffold of smart citizen participation in Dublin, Ireland. GeoJournal, 84, 1–13.\n\
CODEMA 2016. Spatial Energy Demand Analysis. Dublin.\n\
DHINGRA, M., KERR, A. & LEHANE, J. R. Rethinking Digital Twins and Building \n\
Alternatives for Smart City Planning in Dublin. Proceedings of the 60th ISOCARP \n\
World Planning Congress, Toronto, ON, Canada, 2024. 8–12.\n\
RAUSHAN, K., MAC UIDHIR, T., NORTON, B. & AHERN, C. \n\
2024. A data-driven methodology to validate a large dataset. Energy and Buildings, \n\
323, 114774.\n\
SMART DUBLIN 2025. Rethinking Mobility in Ireland. Smart Dublin.\n\
HAQUE, R., CONCHUBHAIR, D. Ó., RAZZAK, M. A., ZELETI, F. A., \n\
DERGUECH, W. & CURRY, E. 2026. Toward the Irish Mobility Data Space. AI and \n\
Robotics, 305–342.\n";

    #[test]
    fn segments_unparenthesised_caps_author_date() {
        let bib = detect(CAPS_AUTHOR_DATE);
        assert!(bib.detected);
        // One entry per reference: CARDULLO, CODEMA, DHINGRA, RAUSHAN, SMART
        // DUBLIN, HAQUE.
        assert_eq!(bib.entries.len(), 6);
        // A year that wrapped to its own line must NOT start a new entry.
        let raushan = bib
            .entries
            .iter()
            .find(|e| e.raw_text.contains("RAUSHAN"))
            .expect("RAUSHAN should be one entry");
        assert!(raushan.raw_text.contains("2024"));
        assert!(raushan.raw_text.contains("114774"));
        // An organisation author with a bare year is its own entry.
        assert!(
            bib.entries
                .iter()
                .any(|e| e.raw_text.starts_with("CODEMA 2016"))
        );
        // An author list that wraps (so a continuation line starts with another
        // "SURNAME, I.") stays a single entry.
        let haque = bib
            .entries
            .iter()
            .find(|e| e.raw_text.starts_with("HAQUE"))
            .expect("HAQUE should be one entry");
        assert!(haque.raw_text.contains("DERGUECH"));
        assert!(haque.raw_text.contains("305"));
    }

    // MLA/Chicago author-title references: the author's given name is spelled out
    // in full and the year is unparenthesised and appears late, so the only entry
    // boundary signal is the author block followed by a (usually quoted) title.
    // Modelled on a real term paper whose list collapsed into one clump (only the
    // first reference, with initials, split off). Curly quotes as pdfium emits.
    const MLA_AUTHOR_TITLE: &str = "References\n\
Arnstein, Sherry R. \u{201c}A Ladder of Citizen Participation.\u{201d} Journal of the American Institute of\n\
Planners, vol. 35, no. 4, 1969, pp. 216\u{2013}24,\n\
https://doi.org/10.1080/01944366908977225.\n\
Batty, M., Axhausen, K.W., Giannotti, F. et al. Smart cities of the future. Eur. Phys. J. Spec.\n\
Top. 214, 481\u{2013}518 (2012). https://doi.org/10.1140/epjst/e2012-01703-3\n\
Chung, Hiu Fung. \u{201c}Changing Repertoires of Contention in Hong Kong: A Case Study on the\n\
Anti-Extradition Bill Movement.\u{201d} China Perspectives, vol. 2020, no. 3, Sept. 2020,\n\
pp. 57\u{2013}63, https://doi.org/10.4000/chinaperspectives.10476.\n\
Cole, Alistair, and \u{c9}milie Tran. \u{201c}Trust and the Smart City: The Hong Kong Paradox.\u{201d} China\n\
Perspectives, Sept. 2022, pp. 9\u{2013}20, https://doi.org/10.4000/chinaperspectives.14039.\n\
Cugurullo, Federico. FRANKENSTEIN URBANISM: Eco, Smart and Autonomous Cities,\n\
Artificial Intelligence and the End of the City. Routledge, 2021.\n\
Vanolo, Alberto. \u{201c}Smartmentality: The Smart City as Disciplinary Strategy.\u{201d} Urban Studies,\n\
vol. 51, no. 5, July 2014, pp. 883\u{2013}98, https://doi.org/10.1177/0042098013494427.\n";

    #[test]
    fn segments_mla_author_title_entries() {
        let bib = detect(MLA_AUTHOR_TITLE);
        assert!(bib.detected);
        // One entry per reference: Arnstein, Batty, Chung, Cole, Cugurullo, Vanolo.
        assert_eq!(bib.entries.len(), 6);

        // The first reference (which precedes any initials-style opener) must not
        // be dropped, and carries its DOI.
        let arnstein = bib
            .entries
            .iter()
            .find(|e| e.raw_text.contains("Arnstein"))
            .expect("Arnstein should be its own entry");
        assert_eq!(arnstein.doi.as_deref(), Some("10.1080/01944366908977225"));

        // The Batty reference must stay whole: its wrapped "Top. 214, 481-518
        // (2012)" continuation must not be mistaken for a new author-date entry.
        let batty = bib
            .entries
            .iter()
            .find(|e| e.raw_text.contains("Batty"))
            .expect("Batty should be one entry");
        assert!(batty.raw_text.contains("Top. 214"));
        assert_eq!(batty.doi.as_deref(), Some("10.1140/epjst/e2012-01703-3"));

        // Adjacent MLA references must not clump together.
        let chung = bib
            .entries
            .iter()
            .find(|e| e.raw_text.contains("Chung"))
            .expect("Chung should be its own entry");
        assert!(!chung.raw_text.contains("Cole"));
        assert_eq!(
            chung.doi.as_deref(),
            Some("10.4000/chinaperspectives.10476")
        );

        // A book/report whose title is not quoted is still its own entry, and its
        // wrapped continuation is not split off.
        let cugurullo = bib
            .entries
            .iter()
            .find(|e| e.raw_text.contains("Cugurullo"))
            .expect("Cugurullo should be its own entry");
        assert!(cugurullo.raw_text.contains("Routledge"));
        assert!(!cugurullo.raw_text.contains("Vanolo"));
    }

    // A page footer that is not page-number shaped (an author name and an ID
    // number) repeats on every page and must be dropped, not glued onto the
    // adjacent reference.
    #[test]
    fn strips_repeated_non_numeric_footer_lines() {
        let text = "References\n\
Smith, J. (2020). Paper one. https://doi.org/10.1/a\n\
Robert Hynes\n\
16321228\n\
Jones, K. (2021). Paper two. https://doi.org/10.2/b\n\
Robert Hynes\n\
16321228\n\
Lee, M. (2022). Paper three. https://doi.org/10.3/c\n\
Robert Hynes\n\
16321228\n";
        let bib = detect(text);
        assert!(bib.detected);
        assert_eq!(bib.entries.len(), 3);
        assert!(
            bib.entries
                .iter()
                .all(|e| !e.raw_text.contains("Robert Hynes") && !e.raw_text.contains("16321228")),
            "repeated footer lines must be stripped, not glued onto references"
        );
    }

    // A URL split across a line wrap must be rejoined without a space, so the
    // link stays intact, while a following access note stays separated.
    #[test]
    fn rejoins_url_split_across_lines() {
        let section = "References\n\
Ackerman, D. (2022) Is my lawn bad for the climate? Available at: \n\
  https://open.spotify.com/episode/abc123?si=yvWZyJ1dQiaRP\n\
  yEefMMNfQ (Accessed September 8, 2022). \n";
        let entries = split_entries(section_after_heading(section).unwrap());
        assert_eq!(entries.len(), 1);
        assert!(
            entries[0].contains("si=yvWZyJ1dQiaRPyEefMMNfQ"),
            "URL should be rejoined without a space: {}",
            entries[0]
        );
        assert!(entries[0].contains("yEefMMNfQ (Accessed"));
    }

    // A URL that wraps right after a path slash (with a trailing space on the
    // first line) must also be rejoined without a space.
    #[test]
    fn rejoins_url_split_after_slash() {
        let section = "References\n\
Bessner, D. (2026) Bonus. Available at: https://open.spotify.com/ \n\
  episode/0V9RrZ?si=HTck0Bu (Accessed Jan 25, 2026). \n";
        let entries = split_entries(section_after_heading(section).unwrap());
        assert_eq!(entries.len(), 1);
        assert!(
            entries[0].contains("https://open.spotify.com/episode/0V9RrZ?si=HTck0Bu"),
            "URL should be rejoined without a space: {}",
            entries[0]
        );
    }

    // A heading with a trailing colon ("Bibliography:") must still be detected.
    #[test]
    fn detects_heading_with_trailing_colon() {
        let text =
            "Body text.\n\nBibliography:\nSmith, J. (2020) A study of things. Journal of Things.\n";
        let bib = detect(text);
        assert!(bib.detected);
        assert_eq!(bib.entries.len(), 1);
        assert!(bib.entries[0].raw_text.contains("Smith"));
    }

    // "Reference List" is a valid heading synonym.
    #[test]
    fn detects_reference_list_heading() {
        let text = "Body.\n\nReference List\nSmith, J. (2020) A study. Journal.\n";
        assert!(detect(text).detected);
    }

    // The references section must stop at a later appendix, not absorb its
    // numbered questions as references.
    #[test]
    fn section_stops_at_appendix() {
        let text = "Bibliography:\n\
Smith, J. (2020) A study of things. Journal of Things.\n\
Appendix A - Interview questions\n\
1. What is your role?\n\
2. What do you think?\n";
        let bib = detect(text);
        assert!(bib.detected);
        assert_eq!(bib.entries.len(), 1);
        assert!(bib.entries[0].raw_text.contains("Smith"));
        assert!(!bib.entries.iter().any(|e| e.raw_text.contains("your role")));
    }

    // A DECLARATION section after the last reference must not be absorbed into it.
    #[test]
    fn section_stops_at_declaration() {
        let text = "References\n\
Yi, H. and Lim, J. (2020) 'Health equity'. Journal of Travel Medicine. https://doi.org/10.1093/jtm/taaa159\n\
DECLARATION\n\
AI tools were used to proofread for grammar.\n\
Name: A Student\n";
        let bib = detect(text);
        assert!(bib.detected);
        let last = bib.entries.last().unwrap();
        assert!(last.raw_text.contains("Yi"));
        assert!(!last.raw_text.contains("DECLARATION"));
        assert!(!last.raw_text.to_lowercase().contains("proofread"));
        assert_eq!(last.doi.as_deref(), Some("10.1093/jtm/taaa159"));
    }

    // A reference whose title begins "Declaration of ..." is NOT a heading and
    // must not truncate the list.
    #[test]
    fn reference_titled_declaration_not_truncated() {
        let text = "References\n\
Declaration of Helsinki (2013) 'Ethical principles'. https://doi.org/10.1001/jama.2013.281053\n\
Zull, A. (2020) 'Last ref'. https://doi.org/10.1000/last\n";
        let bib = detect(text);
        assert!(bib.detected);
        assert_eq!(bib.entries.len(), 2);
        assert!(
            bib.entries
                .iter()
                .any(|e| e.doi.as_deref() == Some("10.1000/last"))
        );
    }

    // "Resources"/"Sources" are heading synonyms used by some student papers.
    #[test]
    fn detects_resources_and_sources_headings() {
        let r = detect("Body.\n\nResources\nSmith, J. (2020) A study. https://doi.org/10.1000/x\n");
        assert!(r.detected);
        let s = detect("Body.\n\nSources\nSmith, J. (2020) A study. https://doi.org/10.1000/x\n");
        assert!(s.detected);
    }

    // Under a recognised heading, adjacent author-date references split apart
    // even when the first carries no DOI.
    #[test]
    fn resources_heading_splits_adjacent_references() {
        let text = "Resources\n\
Atkinson, R. & Easthope, H. (2008) 'Creative Class'. https://www.jstor.org/stable/23289786\n\
Black, J. (2026) 'Towards Net-Zero'. https://doi.org/10.7916/qbtt-xa42\n";
        let bib = detect(text);
        assert!(bib.detected);
        assert_eq!(bib.entries.len(), 2);
        assert!(bib.entries[0].raw_text.contains("Atkinson"));
        assert!(bib.entries[1].raw_text.contains("Black"));
        assert_eq!(bib.entries[1].doi.as_deref(), Some("10.7916/qbtt-xa42"));
    }

    // Repeating headers/page numbers are removed; one-offs and 4-digit numbers
    // are kept.
    #[test]
    fn strip_running_heads_keeps_one_offs() {
        let text = "Anderson 1\nAnderson 2\nAnderson 3\nSmart Cities Plan 2016\nReport 7\n1\n2\n3\nBody text here.\n";
        let cleaned = strip_running_heads(text);
        assert!(!cleaned.contains("Anderson"));
        assert!(cleaned.contains("Smart Cities Plan 2016"));
        assert!(cleaned.contains("Report 7"));
        assert!(cleaned.contains("Body text here."));
        assert!(!cleaned.lines().any(|l| l.trim() == "1"));
    }

    // A recurring running header between references is not glued onto an entry.
    #[test]
    fn running_header_not_glued_to_reference() {
        let text = "References\n\
Albino, V. (2015) 'Smart Cities'. https://doi.org/10.1000/albino\n\
Anderson 9\n\
Atkinson, R. (2008) 'Creative Class'. https://www.jstor.org/stable/23289786\n\
Anderson 10\n\
Black, J. (2026) 'Net-Zero'. https://doi.org/10.1000/black\n\
Anderson 11\n";
        let bib = detect(text);
        assert!(bib.detected);
        assert!(bib.entries.iter().all(|e| !e.raw_text.contains("Anderson")));
    }

    // A bare page number after a DOI must not be glued onto and corrupt the DOI.
    #[test]
    fn page_number_does_not_corrupt_doi() {
        let text = "References\n\
Aaa, B. (2019) 'First'. https://doi.org/10.1000/aaa\n\
12\n\
Yi, H. (2020) 'Health equity'. Journal, 28(2), taaa159. https://doi.org/10.1093/jtm/taaa159\n\
13\n\
Zzz, C. (2021) 'Third'. https://doi.org/10.1000/zzz\n\
14\n";
        let bib = detect(text);
        assert!(bib.detected);
        let yi = bib
            .entries
            .iter()
            .find(|e| e.raw_text.contains("Yi"))
            .unwrap();
        assert_eq!(yi.doi.as_deref(), Some("10.1093/jtm/taaa159"));
        assert!(!yi.raw_text.contains("taaa15914"));
    }
}
