//! Detect markers in reference text that indicate the citation was taken from an
//! LLM/chatbot. The strongest signal is a `utm_source=chatgpt.com` tracking
//! parameter that ChatGPT (and similar tools) append to links they serve. This
//! is an academic-integrity signal, not a correctness one.

/// LLM/chatbot source markers (lowercase), most specific first.
const MARKERS: &[&str] = &[
    "utm_source=chatgpt.com",
    "utm_source=openai",
    "utm_source=perplexity",
    "utm_source=copilot",
    "utm_source=gemini",
    "utm_source=claude",
    "chat.openai.com",
    "chatgpt.com",
    "perplexity.ai",
    "copilot.microsoft.com",
    "gemini.google.com",
    "claude.ai",
    "poe.com",
];

/// The first LLM-source marker found in `text`, if any. Matching ignores case and
/// whitespace, so a marker split across a PDF line wrap is still detected.
pub fn llm_source(text: &str) -> Option<String> {
    let collapsed: String = text
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect();
    MARKERS
        .iter()
        .find(|m| collapsed.contains(*m))
        .map(|m| m.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_chatgpt_utm_source() {
        let t =
            "Smith (2024) Report. Available at: https://example.com/x.pdf?utm_source=chatgpt.com";
        assert_eq!(llm_source(t).as_deref(), Some("utm_source=chatgpt.com"));
    }

    #[test]
    fn detects_across_whitespace() {
        assert_eq!(
            llm_source("...x.pdf?utm_source=chat gpt.com").as_deref(),
            Some("utm_source=chatgpt.com")
        );
    }

    #[test]
    fn clean_reference_is_none() {
        assert_eq!(
            llm_source("Smith (2024). A study. https://doi.org/10.1/x"),
            None
        );
    }
}
