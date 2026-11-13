//! The HTTP client a [`RigBackend`](super::RigBackend) sends through: reqwest
//! with exponential-backoff retry and tracing middleware.

use std::time::Duration;

use elide_core::{Error, Result};
use reqwest_middleware::ClientBuilder;
use reqwest_middleware::reqwest::Client;
use reqwest_retry::RetryTransientMiddleware;
use reqwest_retry::policies::ExponentialBackoff;
use reqwest_tracing::TracingMiddleware;
use rig_reqwest::ReqwestMiddlewareClient;

const TARGET: &str = "elide_llm::http";

/// Per-request timeout.
const TIMEOUT: Duration = Duration::from_secs(120);
/// TCP connection timeout.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Keep-alive pool idle timeout.
const IDLE_TIMEOUT: Duration = Duration::from_secs(90);

/// A rig HTTP client that retries a transient failure up to `max_retries`
/// times, with exponential backoff, and traces every request.
///
/// # Errors
///
/// A configuration error when the underlying `reqwest::Client` cannot be
/// built (e.g. TLS backend initialisation failure).
pub(super) fn http_client(max_retries: u32) -> Result<ReqwestMiddlewareClient> {
    tracing::debug!(target: TARGET, max_retries, "building HTTP client");
    let client = Client::builder()
        .timeout(TIMEOUT)
        .connect_timeout(CONNECT_TIMEOUT)
        .pool_idle_timeout(IDLE_TIMEOUT)
        .build()
        .map_err(|e| Error::configuration(format!("building HTTP client: {e}")))?;
    let policy = ExponentialBackoff::builder().build_with_max_retries(max_retries);
    let middleware = ClientBuilder::new(client)
        .with(TracingMiddleware::default())
        .with(RetryTransientMiddleware::new_with_policy(policy))
        .build();
    Ok(ReqwestMiddlewareClient::from(middleware))
}
