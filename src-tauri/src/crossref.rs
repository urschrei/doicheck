//! Async Crossref client: resolve a DOI, and search by bibliographic text.

use crate::compare::Metadata;
use serde::Deserialize;

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
            title: self.title.first().cloned(),
            first_author_surname: self
                .author
                .first()
                .map(|a| a.family.clone())
                .filter(|f| !f.is_empty()),
            year: self
                .issued
                .as_ref()
                .and_then(|i| i.date_parts.first())
                .and_then(|p| p.first())
                .copied(),
            container_title: self.container_title.first().cloned(),
        }
    }
}

pub struct SearchHit {
    pub doi: String,
    pub metadata: Metadata,
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
        }
    }

    /// Override the API base URL (used by tests and for configurability).
    pub fn with_base(email: &str, base: String) -> Self {
        let mut c = Self::new(email);
        c.base = base;
        c
    }

    pub async fn resolve(&self, doi: &str) -> Result<Metadata, CrossrefError> {
        let url = format!("{}/works/{}", self.base, urlencoding::encode(doi));
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| CrossrefError::Network(e.to_string()))?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(CrossrefError::NotFound);
        }
        let body: WorkMessage = resp
            .error_for_status()
            .map_err(|e| CrossrefError::Network(e.to_string()))?
            .json()
            .await
            .map_err(|e| CrossrefError::Network(e.to_string()))?;
        Ok(body.message.to_metadata())
    }

    pub async fn search(&self, reference: &str) -> Result<Option<SearchHit>, CrossrefError> {
        let url = format!("{}/works", self.base);
        let resp = self
            .http
            .get(&url)
            .query(&[("query.bibliographic", reference), ("rows", "1")])
            .send()
            .await
            .map_err(|e| CrossrefError::Network(e.to_string()))?;
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
