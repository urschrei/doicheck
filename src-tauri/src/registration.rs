//! Look up which agency registered a DOI via the doi.org registration-agency
//! endpoint (`/doiRA/<doi>`). This tells a genuinely unregistered DOI (often a
//! non-DOI identifier miscast as one, such as a JSTOR stable id) apart from a
//! valid DOI that Crossref and DataCite simply do not index (mEDRA, JaLC, and
//! other agencies). Used only once both metadata clients have returned a
//! definitive 404, so it runs on the minority failure path.

use crate::lookup::{LookupError, send_with_retry};
use crate::model::Registration;
use serde::Deserialize;
use std::time::Duration;

/// One element of the `/doiRA` JSON array. A registered DOI carries `RA`; an
/// unregistered one carries `status` (e.g. "DOI does not exist", "Invalid DOI").
#[derive(Deserialize)]
struct RaEntry {
    #[serde(rename = "RA")]
    ra: Option<String>,
}

pub struct RegistrationClient {
    http: reqwest::Client,
    base: String,
    max_retries: u32,
    base_delay: Duration,
}

impl RegistrationClient {
    /// `email` is included in the User-Agent, matching the metadata clients.
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
            base: "https://doi.org".to_string(),
            max_retries: 4,
            base_delay: Duration::from_millis(500),
        }
    }

    /// Override the base URL (used by tests). Sets `base_delay` to zero so
    /// retries are instant.
    pub fn with_base(email: &str, base: String) -> Self {
        let mut c = Self::new(email);
        c.base = base;
        c.base_delay = Duration::ZERO;
        c
    }

    /// Classify a DOI by querying `/doiRA/<doi>`. The endpoint answers 200 with a
    /// JSON array even for an unregistered DOI, so the body, not the status,
    /// carries the answer. A network failure (or an unparseable body, which we
    /// treat as not knowing) propagates so the caller keeps the entry retryable
    /// rather than wrongly reporting it unregistered.
    pub async fn check(&self, doi: &str) -> Result<Registration, LookupError> {
        // The DOI sits in the URL path, where its prefix/suffix slash must stay
        // a literal separator (doi.org does not accept a percent-encoded slash
        // here), so the DOI is not encoded.
        let url = format!("{}/doiRA/{}", self.base, doi);
        let resp =
            send_with_retry(self.max_retries, self.base_delay, || self.http.get(&url)).await?;
        let body = resp
            .text()
            .await
            .map_err(|e| LookupError::Network(e.to_string()))?;
        let entries: Vec<RaEntry> = serde_json::from_str(&body)
            .map_err(|e| LookupError::Network(format!("doiRA parse: {e}")))?;
        match entries.into_iter().next() {
            Some(RaEntry { ra: Some(ra) }) if !ra.trim().is_empty() => Ok(Registration::Agency(ra)),
            _ => Ok(Registration::Unregistered),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn server_with(doi: &str, body: serde_json::Value) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!("/doiRA/{doi}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;
        server
    }

    #[tokio::test]
    async fn unregistered_doi_is_reported_unregistered() {
        let doi = "10.2307/26428370";
        let server = server_with(
            doi,
            serde_json::json!([{ "DOI": doi, "status": "DOI does not exist" }]),
        )
        .await;
        let client = RegistrationClient::with_base("", server.uri());
        assert_eq!(client.check(doi).await.unwrap(), Registration::Unregistered);
    }

    #[tokio::test]
    async fn doi_registered_with_another_agency_names_it() {
        let doi = "10.5072/example";
        let server = server_with(doi, serde_json::json!([{ "DOI": doi, "RA": "mEDRA" }])).await;
        let client = RegistrationClient::with_base("", server.uri());
        assert_eq!(
            client.check(doi).await.unwrap(),
            Registration::Agency("mEDRA".to_string())
        );
    }

    #[tokio::test]
    async fn an_unparseable_body_is_a_network_error_not_unregistered() {
        let doi = "10.1/x";
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!("/doiRA/{doi}")))
            .respond_with(ResponseTemplate::new(200).set_body_string("<html>down</html>"))
            .mount(&server)
            .await;
        let client = RegistrationClient::with_base("", server.uri());
        assert!(matches!(
            client.check(doi).await,
            Err(LookupError::Network(_))
        ));
    }
}
