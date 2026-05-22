//! Async Crossref client: resolve a DOI, and search by bibliographic text.

use crate::compare::Metadata;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Resolve XML/HTML entity references that Crossref sometimes returns in string
/// fields (e.g. `&amp;`, `&#233;`). Falls back to the raw string on error.
fn unescape(s: &str) -> String {
    quick_xml::escape::unescape(s)
        .map(|c| c.into_owned())
        .unwrap_or_else(|_| s.to_string())
}

pub use crate::lookup::LookupError;
use crate::lookup::send_with_retry;

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

#[derive(Debug, Deserialize, Serialize)]
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

#[derive(Debug, Deserialize, Serialize)]
struct Author {
    #[serde(default)]
    family: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct Issued {
    #[serde(rename = "date-parts", default)]
    date_parts: Vec<Vec<i32>>,
}

/// Re-serialises a matched search work in the `{ "message": <work> }` shape that
/// `resolve_json` caches, so a search hit can seed the DOI cache.
#[derive(Serialize)]
struct WorkRecord<'a> {
    message: &'a Work,
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
    /// The matched work as `{ "message": <work> }` JSON, ready to seed the DOI
    /// cache so a later direct resolve of `doi` is a cache hit.
    pub record: String,
}

impl CrossrefClient {
    /// `email` is included in the User-Agent for the Crossref polite pool.
    pub fn new(email: &str) -> Self {
        let ua = if email.trim().is_empty() {
            "doicheck/0.1".to_string()
        } else {
            format!("doicheck/0.1 (mailto:{})", email.trim())
        };
        // Building the client only fails if the TLS backend cannot initialise,
        // which is an unrecoverable environment failure; there is no infallible
        // reqwest constructor, so panic here rather than thread a Result through
        // every caller.
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

    /// Fetch the raw JSON body for a DOI from the Crossref `/works/{doi}` endpoint.
    pub async fn resolve_json(&self, doi: &str) -> Result<String, LookupError> {
        let url = format!("{}/works/{}", self.base, urlencoding::encode(doi));
        let resp =
            send_with_retry(self.max_retries, self.base_delay, || self.http.get(&url)).await?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(LookupError::NotFound);
        }
        resp.error_for_status()
            .map_err(|e| LookupError::Network(e.to_string()))?
            .text()
            .await
            .map_err(|e| LookupError::Network(e.to_string()))
    }

    pub async fn resolve(&self, doi: &str) -> Result<Metadata, LookupError> {
        let body = self.resolve_json(doi).await?;
        Ok(metadata_from_json(&body))
    }

    pub async fn search(&self, reference: &str) -> Result<Option<SearchHit>, LookupError> {
        let url = format!("{}/works", self.base);
        let resp = send_with_retry(self.max_retries, self.base_delay, || {
            self.http
                .get(&url)
                .query(&[("query.bibliographic", reference), ("rows", "1")])
        })
        .await?;
        let body: SearchMessage = resp
            .error_for_status()
            .map_err(|e| LookupError::Network(e.to_string()))?
            .json()
            .await
            .map_err(|e| LookupError::Network(e.to_string()))?;
        Ok(body.message.items.into_iter().next().and_then(|w| {
            if w.doi.is_empty() {
                return None;
            }
            let record = serde_json::to_string(&WorkRecord { message: &w }).ok()?;
            let metadata = w.to_metadata();
            Some(SearchHit {
                doi: w.doi,
                metadata,
                record,
            })
        }))
    }
}

/// Parse a Crossref `/works/{doi}` response body into comparison metadata. A
/// parse failure (corrupt cache entry or an API contract change) is logged and
/// treated as empty metadata.
pub fn metadata_from_json(body: &str) -> Metadata {
    match serde_json::from_str::<WorkMessage>(body) {
        Ok(m) => m.message.to_metadata(),
        Err(e) => {
            log::warn!("crossref: failed to parse metadata JSON ({e}); treating as empty");
            Metadata::default()
        }
    }
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

    // The `record` a search hit carries (to seed the DOI cache) must parse back
    // through `metadata_from_json` exactly like a resolved record.
    #[test]
    fn search_record_round_trips_through_metadata_from_json() {
        let work = Work {
            title: vec!["A Study of Widgets".to_string()],
            author: vec![Author {
                family: "Smith".to_string(),
            }],
            container_title: vec!["Journal of Widgets".to_string()],
            issued: Some(Issued {
                date_parts: vec![vec![2020]],
            }),
            doi: "10.1000/x".to_string(),
        };
        let record = serde_json::to_string(&WorkRecord { message: &work }).unwrap();
        let m = metadata_from_json(&record);
        assert_eq!(m.title.as_deref(), Some("A Study of Widgets"));
        assert_eq!(m.first_author_surname.as_deref(), Some("Smith"));
        assert_eq!(m.year, Some(2020));
        assert_eq!(m.container_title.as_deref(), Some("Journal of Widgets"));
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
