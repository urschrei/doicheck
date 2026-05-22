//! Async DataCite client: resolve a DOI, and search by free text. DataCite is
//! the registration agency for datasets, preprints, theses, software and most
//! repository content, so it backs DOIs that Crossref does not index.

use crate::compare::Metadata;
use crate::crossref::SearchHit;
use crate::lookup::{LookupError, send_with_retry};
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Deserialize)]
struct Single {
    data: Record,
}

#[derive(Debug, Deserialize)]
struct Many {
    data: Vec<Record>,
}

#[derive(Debug, Deserialize)]
struct Record {
    attributes: Attributes,
}

#[derive(Debug, Deserialize, Serialize, Default)]
struct Attributes {
    #[serde(default)]
    doi: String,
    #[serde(default)]
    titles: Vec<Title>,
    #[serde(default)]
    creators: Vec<Creator>,
    #[serde(rename = "publicationYear", default)]
    publication_year: Option<i32>,
    #[serde(default)]
    container: Container,
}

#[derive(Debug, Deserialize, Serialize, Default)]
struct Title {
    #[serde(default)]
    title: String,
}

#[derive(Debug, Deserialize, Serialize, Default)]
struct Creator {
    #[serde(default)]
    name: String,
    #[serde(rename = "familyName", default)]
    family_name: Option<String>,
    #[serde(rename = "nameType", default)]
    name_type: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Default)]
struct Container {
    #[serde(default)]
    title: Option<String>,
}

/// Re-serialises a matched search record in the `{ "data": { "attributes": .. } }`
/// shape that `resolve_json` returns, so a search hit can seed the DOI cache.
#[derive(Serialize)]
struct Envelope<'a> {
    data: DataNode<'a>,
}

#[derive(Serialize)]
struct DataNode<'a> {
    attributes: &'a Attributes,
}

impl Creator {
    /// A personal surname for comparison: prefer `familyName`; otherwise take the
    /// part before the comma in a "Family, Given" `name`. Organisational creators
    /// have no personal surname.
    fn surname(&self) -> Option<String> {
        if let Some(f) = self.family_name.as_deref().filter(|s| !s.is_empty()) {
            return Some(f.to_string());
        }
        if self.name_type.as_deref() == Some("Organizational") {
            return None;
        }
        let surname = self.name.split(',').next().unwrap_or("").trim();
        (!surname.is_empty()).then(|| surname.to_string())
    }
}

impl Attributes {
    fn to_metadata(&self) -> Metadata {
        Metadata {
            title: self
                .titles
                .first()
                .map(|t| t.title.clone())
                .filter(|s| !s.is_empty()),
            first_author_surname: self.creators.first().and_then(Creator::surname),
            year: self.publication_year,
            // DataCite `container` is usually empty for datasets; `publisher`
            // (e.g. "Zenodo") is not a journal/container, so it is left unmapped
            // rather than producing false container mismatches.
            container_title: self.container.title.clone().filter(|s| !s.is_empty()),
        }
    }
}

#[derive(Clone)]
pub struct DataCiteClient {
    http: reqwest::Client,
    base: String,
    max_retries: u32,
    base_delay: Duration,
}

impl DataCiteClient {
    /// `email` is included in the User-Agent as a courtesy to the API.
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
            base: "https://api.datacite.org".to_string(),
            max_retries: 4,
            base_delay: Duration::from_millis(500),
        }
    }

    /// Override the API base URL (used by tests). Sets `base_delay` to zero so
    /// retries are instant in tests.
    pub fn with_base(email: &str, base: String) -> Self {
        let mut c = Self::new(email);
        c.base = base;
        c.base_delay = Duration::ZERO;
        c
    }

    /// Fetch the raw JSON body for a DOI from the DataCite `/dois/{doi}` endpoint.
    pub async fn resolve_json(&self, doi: &str) -> Result<String, LookupError> {
        let url = format!("{}/dois/{}", self.base, urlencoding::encode(doi));
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

    /// Search DataCite by free text, returning the top hit (if any).
    pub async fn search(&self, reference: &str) -> Result<Option<SearchHit>, LookupError> {
        let url = format!("{}/dois", self.base);
        let resp = send_with_retry(self.max_retries, self.base_delay, || {
            self.http
                .get(&url)
                .query(&[("query", reference), ("page[size]", "1")])
        })
        .await?;
        let body: Many = resp
            .error_for_status()
            .map_err(|e| LookupError::Network(e.to_string()))?
            .json()
            .await
            .map_err(|e| LookupError::Network(e.to_string()))?;
        Ok(body.data.into_iter().next().and_then(|rec| {
            let attrs = rec.attributes;
            if attrs.doi.is_empty() {
                return None;
            }
            let record = serde_json::to_string(&Envelope {
                data: DataNode { attributes: &attrs },
            })
            .ok()?;
            let metadata = attrs.to_metadata();
            Some(SearchHit {
                doi: attrs.doi,
                metadata,
                record,
            })
        }))
    }
}

/// Parse a DataCite `/dois/{doi}` response body into comparison metadata. A parse
/// failure (corrupt cache entry or an API contract change) is logged and treated
/// as empty metadata.
pub fn metadata_from_json(body: &str) -> Metadata {
    match serde_json::from_str::<Single>(body) {
        Ok(s) => s.data.attributes.to_metadata(),
        Err(e) => {
            log::warn!("datacite: failed to parse metadata JSON ({e}); treating as empty");
            Metadata::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path_regex, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn attrs(json: &str) -> Metadata {
        serde_json::from_str::<Attributes>(json)
            .unwrap()
            .to_metadata()
    }

    #[test]
    fn maps_personal_creator_with_family_name() {
        let m = attrs(
            r#"{"doi":"10.5281/zenodo.1","titles":[{"title":"A Dataset"}],
                "creators":[{"name":"Hannah, Thiel","familyName":"Hannah","nameType":"Personal"}],
                "publicationYear":2023,"container":{}}"#,
        );
        assert_eq!(m.title.as_deref(), Some("A Dataset"));
        assert_eq!(m.first_author_surname.as_deref(), Some("Hannah"));
        assert_eq!(m.year, Some(2023));
        assert_eq!(m.container_title, None);
    }

    #[test]
    fn falls_back_to_name_before_comma_when_family_name_absent() {
        let m = attrs(
            r#"{"doi":"10.1/x","titles":[{"title":"T"}],
                "creators":[{"name":"Charitonidou, Marianna","nameType":"Personal"}]}"#,
        );
        assert_eq!(m.first_author_surname.as_deref(), Some("Charitonidou"));
    }

    #[test]
    fn skips_organisational_creator_surname() {
        let m = attrs(
            r#"{"doi":"10.1/x","titles":[{"title":"T"}],
                "creators":[{"name":"Central Statistics Office","nameType":"Organizational"}]}"#,
        );
        assert_eq!(m.first_author_surname, None);
    }

    #[test]
    fn maps_container_title_when_present() {
        let m =
            attrs(r#"{"doi":"10.1/x","titles":[{"title":"T"}],"container":{"title":"Journal X"}}"#);
        assert_eq!(m.container_title.as_deref(), Some("Journal X"));
    }

    // The `record` a search hit carries (to seed the DOI cache) must parse back
    // through `metadata_from_json` like a resolved record.
    #[tokio::test]
    async fn search_hit_record_round_trips_through_metadata_from_json() {
        let server = MockServer::start().await;
        let body = serde_json::json!({
            "data": [{
                "attributes": {
                    "doi": "10.5281/zenodo.42",
                    "titles": [{"title": "Smart City Digital Twins"}],
                    "creators": [{"name": "Lee, Kim", "familyName": "Lee", "nameType": "Personal"}],
                    "publicationYear": 2022,
                    "container": {}
                }
            }]
        });
        Mock::given(method("GET"))
            .and(query_param("query", "smart city"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;
        let client = DataCiteClient::with_base("", server.uri());

        let hit = client.search("smart city").await.unwrap().unwrap();
        assert_eq!(hit.doi, "10.5281/zenodo.42");
        let m = metadata_from_json(&hit.record);
        assert_eq!(m.title.as_deref(), Some("Smart City Digital Twins"));
        assert_eq!(m.first_author_surname.as_deref(), Some("Lee"));
        assert_eq!(m.year, Some(2022));
    }

    #[tokio::test]
    async fn resolve_retries_on_503_then_succeeds() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(503))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path_regex(r"/dois/.*"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"data":{"attributes":{"doi":"10.1/x"}}})),
            )
            .mount(&server)
            .await;
        let client = DataCiteClient::with_base("", server.uri());
        let body = client.resolve_json("10.1/x").await.unwrap();
        assert!(body.contains("10.1/x"));
    }

    #[tokio::test]
    async fn resolve_maps_404_to_not_found() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        let client = DataCiteClient::with_base("", server.uri());
        assert!(matches!(
            client.resolve_json("10.1/missing").await,
            Err(LookupError::NotFound)
        ));
    }
}
