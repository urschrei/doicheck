//! Shared HTTP plumbing for the metadata-lookup clients (Crossref, DataCite):
//! a common error type and a retrying request sender, so both clients share one
//! retry/backoff policy and one error vocabulary.

use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum LookupError {
    #[error("network error: {0}")]
    Network(String),
    #[error("not found")]
    NotFound,
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

/// Exponential backoff duration for `attempt` (0-indexed) given a base delay.
fn backoff(base_delay: Duration, attempt: u32) -> Duration {
    base_delay.saturating_mul(2u32.saturating_pow(attempt))
}

/// Send a request built by `build`, retrying on HTTP 429/5xx and send errors,
/// up to `max_retries` times with exponential backoff. Retries exhausted on a
/// transient status become a `Network` error so the result is not cached and can
/// be re-checked later.
pub async fn send_with_retry(
    max_retries: u32,
    base_delay: Duration,
    build: impl Fn() -> reqwest::RequestBuilder,
) -> Result<reqwest::Response, LookupError> {
    let mut attempt: u32 = 0;
    loop {
        match build().send().await {
            Ok(resp) => {
                let s = resp.status();
                let transient = s == reqwest::StatusCode::TOO_MANY_REQUESTS || s.is_server_error();
                if transient {
                    if attempt < max_retries {
                        let delay =
                            retry_after(&resp).unwrap_or_else(|| backoff(base_delay, attempt));
                        attempt += 1;
                        tokio::time::sleep(delay).await;
                        continue;
                    }
                    return Err(LookupError::Network(format!(
                        "server returned {s} after {max_retries} retries"
                    )));
                }
                return Ok(resp);
            }
            Err(e) => {
                if attempt < max_retries {
                    let delay = backoff(base_delay, attempt);
                    attempt += 1;
                    tokio::time::sleep(delay).await;
                    continue;
                }
                return Err(LookupError::Network(e.to_string()));
            }
        }
    }
}
