//! Async Crossref client: resolve a DOI, and search by bibliographic text.

use crate::compare::Metadata;
use serde::Deserialize;
use std::time::Duration;

/// Resolve XML/HTML entity references that Crossref sometimes returns in string
/// fields (e.g. `&amp;`, `&#233;`). Falls back to the raw string on error.
fn unescape(s: &str) -> String {
    quick_xml::escape::unescape(s)
        .map(|c| c.into_owned())
        .unwrap_or_else(|_| s.to_string())
}

#[derive(Debug, thiserror::Error)]
pub enum CrossrefError {
    #[error("network error: {0}")]
    Network(String),
    #[error("doi not found")]
    NotFound,
}

#[derive(Clone)]
pub struct CrossrefClient {
    http: reqwest::Client,
    base: String,
    max_retries: u32,
    base_delay: Duration,
}

#[derive(Debug, Deserialize)]
struct WorkMessage {
    message: Work,
}

#[derive(Debug, Deserialize)]
struct SearchMessage {
    message: SearchBody,
}

#[derive(Debug, Deserialize)]
struct SearchBody {
    items: Vec<Work>,
}

#[derive(Debug, Deserialize)]
struct Work {
    #[serde(default)]
    title: Vec<String>,
    #[serde(default)]
    author: Vec<Author>,
    #[serde(default, rename = "container-title")]
    container_title: Vec<String>,
    #[serde(default)]
    issued: Option<Issued>,
    #[serde(rename = "DOI", default)]
    doi: String,
}

#[derive(Debug, Deserialize)]
struct Author {
    #[serde(default)]
    family: String,
}

#[derive(Debug, Deserialize)]
struct Issued {
    #[serde(rename = "date-parts", default)]
    date_parts: Vec<Vec<i32>>,
}

impl Work {
    fn to_metadata(&self) -> Metadata {
        Metadata {
            title: self.title.first().map(|t| unescape(t)),
            first_author_surname: self
                .author
                .first()
                .map(|a| unescape(&a.family))
                .filter(|f| !f.is_empty()),
            year: self
                .issued
                .as_ref()
                .and_then(|i| i.date_parts.first())
                .and_then(|p| p.first())
                .copied(),
            container_title: self.container_title.first().map(|c| unescape(c)),
        }
    }
}

pub struct SearchHit {
    pub doi: String,
    pub metadata: Metadata,
}

/// Read a `Retry-After` header value (seconds) from a response.
fn retry_after(resp: &reqwest::Response) -> Option<Duration> {
    resp.headers()
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .parse::<u64>()
        .ok()
        .map(Duration::from_secs)
}

impl CrossrefClient {
    /// `email` is included in the User-Agent for the Crossref polite pool.
    pub fn new(email: &str) -> Self {
        let ua = if email.trim().is_empty() {
            "doicheck/0.1".to_string()
        } else {
            format!("doicheck/0.1 (mailto:{})", email.trim())
        };
        let http = reqwest::Client::builder()
            .user_agent(ua)
            .build()
            .expect("client builds");
        Self {
            http,
            base: "https://api.crossref.org".to_string(),
            max_retries: 4,
            base_delay: Duration::from_millis(500),
        }
    }

    /// Override the API base URL (used by tests and for configurability).
    /// Sets `base_delay` to zero so retries are instant in tests.
    pub fn with_base(email: &str, base: String) -> Self {
        let mut c = Self::new(email);
        c.base = base;
        c.base_delay = Duration::ZERO;
        c
    }

    /// Exponential backoff duration for `attempt` (0-indexed).
    fn backoff(&self, attempt: u32) -> Duration {
        self.base_delay.saturating_mul(2u32.saturating_pow(attempt))
    }

    /// Send a request built by `build`, retrying on HTTP 429/5xx and send
    /// errors, up to `self.max_retries` times with exponential backoff.
    async fn send_with_retry(
        &self,
        build: impl Fn() -> reqwest::RequestBuilder,
    ) -> Result<reqwest::Response, CrossrefError> {
        let mut attempt: u32 = 0;
        loop {
            match build().send().await {
                Ok(resp) => {
                    let s = resp.status();
                    let transient =
                        s == reqwest::StatusCode::TOO_MANY_REQUESTS || s.is_server_error();
                    if transient && attempt < self.max_retries {
                        let delay = retry_after(&resp).unwrap_or_else(|| self.backoff(attempt));
                        attempt += 1;
                        tokio::time::sleep(delay).await;
                        continue;
                    }
                    return Ok(resp);
                }
                Err(e) => {
                    if attempt < self.max_retries {
                        let delay = self.backoff(attempt);
                        attempt += 1;
                        tokio::time::sleep(delay).await;
                        continue;
                    }
                    return Err(CrossrefError::Network(e.to_string()));
                }
            }
        }
    }

    /// Fetch the raw JSON body for a DOI from the Crossref `/works/{doi}` endpoint.
    pub async fn resolve_json(&self, doi: &str) -> Result<String, CrossrefError> {
        let url = format!("{}/works/{}", self.base, urlencoding::encode(doi));
        let resp = self.send_with_retry(|| self.http.get(&url)).await?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(CrossrefError::NotFound);
        }
        resp.error_for_status()
            .map_err(|e| CrossrefError::Network(e.to_string()))?
            .text()
            .await
            .map_err(|e| CrossrefError::Network(e.to_string()))
    }

    pub async fn resolve(&self, doi: &str) -> Result<Metadata, CrossrefError> {
        let body = self.resolve_json(doi).await?;
        Ok(metadata_from_json(&body))
    }

    pub async fn search(&self, reference: &str) -> Result<Option<SearchHit>, CrossrefError> {
        let url = format!("{}/works", self.base);
        let resp = self
            .send_with_retry(|| {
                self.http
                    .get(&url)
                    .query(&[("query.bibliographic", reference), ("rows", "1")])
            })
            .await?;
        let body: SearchMessage = resp
            .error_for_status()
            .map_err(|e| CrossrefError::Network(e.to_string()))?
            .json()
            .await
            .map_err(|e| CrossrefError::Network(e.to_string()))?;
        Ok(body.message.items.into_iter().next().and_then(|w| {
            if w.doi.is_empty() {
                return None;
            }
            Some(SearchHit {
                metadata: w.to_metadata(),
                doi: w.doi,
            })
        }))
    }
}

/// Parse a Crossref `/works/{doi}` response body into comparison metadata.
pub fn metadata_from_json(body: &str) -> Metadata {
    serde_json::from_str::<WorkMessage>(body)
        .map(|m| m.message.to_metadata())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_metadata_unescapes_html_entities() {
        let work = Work {
            title: vec!["Science, Technology, &amp; Human Values".to_string()],
            author: vec![Author {
                family: "O&apos;Neil".to_string(),
            }],
            container_title: vec!["A &lt;Journal&gt;".to_string()],
            issued: None,
            doi: "10.1000/x".to_string(),
        };
        let m = work.to_metadata();
        assert_eq!(
            m.title.as_deref(),
            Some("Science, Technology, & Human Values")
        );
        assert_eq!(m.first_author_surname.as_deref(), Some("O'Neil"));
        assert_eq!(m.container_title.as_deref(), Some("A <Journal>"));
    }

    #[tokio::test]
    async fn resolve_retries_on_503_then_succeeds() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(503))
            .up_to_n_times(1)
            .with_priority(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "message": { "title": ["Recovered"], "DOI": "10.1000/abc" }
            })))
            .with_priority(2)
            .mount(&server)
            .await;
        let client = CrossrefClient::with_base("", server.uri());
        let meta = client.resolve("10.1000/abc").await.unwrap();
        assert_eq!(meta.title.as_deref(), Some("Recovered"));
    }
}
