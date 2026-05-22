use doicheck_lib::crossref::CrossrefClient;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn resolve_returns_metadata() {
    let server = MockServer::start().await;
    let body = serde_json::json!({
        "message": {
            "title": ["A Study of Widgets"],
            "author": [{"family": "Smith"}],
            "container-title": ["Journal of Widgets"],
            "issued": {"date-parts": [[2020, 5, 1]]},
            "DOI": "10.1000/abc"
        }
    });
    Mock::given(method("GET"))
        .and(path("/works/10.1000%2Fabc"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;

    let client = CrossrefClient::with_base("test@example.com", server.uri());
    let meta = client.resolve("10.1000/abc").await.unwrap();
    assert_eq!(meta.title.as_deref(), Some("A Study of Widgets"));
    assert_eq!(meta.first_author_surname.as_deref(), Some("Smith"));
    assert_eq!(meta.year, Some(2020));
}

#[tokio::test]
async fn resolve_maps_404_to_not_found() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    let client = CrossrefClient::with_base("", server.uri());
    let err = client.resolve("10.1000/missing").await.unwrap_err();
    assert!(matches!(err, doicheck_lib::crossref::LookupError::NotFound));
}

#[tokio::test]
async fn search_returns_top_hit() {
    let server = MockServer::start().await;
    let body = serde_json::json!({
        "message": { "items": [{
            "title": ["A Study of Widgets"],
            "author": [{"family": "Smith"}],
            "DOI": "10.1000/xyz"
        }]}
    });
    Mock::given(method("GET"))
        .and(path("/works"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;
    let client = CrossrefClient::with_base("", server.uri());
    let hit = client
        .search("Smith A Study of Widgets")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(hit.doi, "10.1000/xyz");
    assert_eq!(hit.metadata.title.as_deref(), Some("A Study of Widgets"));
}

#[tokio::test]
async fn search_with_no_items_returns_none() {
    let server = MockServer::start().await;
    let body = serde_json::json!({ "message": { "items": [] } });
    Mock::given(method("GET"))
        .and(path("/works"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;
    let client = CrossrefClient::with_base("", server.uri());
    assert!(client.search("nothing matches").await.unwrap().is_none());
}

#[tokio::test]
async fn search_ignores_hit_with_empty_doi() {
    let server = MockServer::start().await;
    let body = serde_json::json!({ "message": { "items": [{ "title": ["X"], "DOI": "" }] } });
    Mock::given(method("GET"))
        .and(path("/works"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;
    let client = CrossrefClient::with_base("", server.uri());
    assert!(client.search("x").await.unwrap().is_none());
}
