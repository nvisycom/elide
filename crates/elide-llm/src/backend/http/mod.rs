//! HTTP client factory for the rig backend (reqwest + retry + tracing
//! middleware).
//!
//! [`build_http_client`] builds a rig [`ReqwestMiddlewareClient`] from an
//! [`HttpConfig`] with exponential-backoff retry and OpenTelemetry tracing
//! layers pre-installed; the [`RigBackend`] hands the returned client to its
//! rig provider at construction via `with_http`.
//!
//! [`ReqwestMiddlewareClient`]: rig_reqwest::ReqwestMiddlewareClient
//! [`RigBackend`]: super::RigBackend

mod config;
mod middleware;

use elide_core::{Error, ErrorKind, Result};
use reqwest_middleware::ClientBuilder;
use reqwest_middleware::reqwest::Client;
use rig_reqwest::ReqwestMiddlewareClient;

pub use self::config::HttpConfig;
use self::middleware::{backoff_policy, retry_layer, tracing_layer};

const TARGET: &str = "elide_llm::http";

/// Build a rig [`ReqwestMiddlewareClient`] from the given configuration.
///
/// The returned client has exponential-backoff retry and OpenTelemetry
/// tracing middleware pre-installed; the rig backend wires it into the
/// provider via `with_http`.
///
/// # Errors
///
/// Returns an error if the underlying `reqwest::Client` cannot be
/// built (e.g. TLS backend initialisation failure).
pub fn build_http_client(config: &HttpConfig) -> Result<ReqwestMiddlewareClient> {
    tracing::debug!(
        target: TARGET,
        max_retries = config.max_retries,
        timeout = ?config.timeout,
        connect_timeout = ?config.connect_timeout,
        idle_timeout = ?config.idle_timeout,
        "building HTTP client"
    );

    let policy = backoff_policy(config.max_retries);

    let client = Client::builder()
        .timeout(config.timeout)
        .connect_timeout(config.connect_timeout)
        .pool_idle_timeout(config.idle_timeout)
        .build()
        .map_err(|e| {
            Error::new(
                ErrorKind::Configuration,
                format!("failed to build HTTP client: {e}"),
            )
        })?;

    let middleware = ClientBuilder::new(client)
        .with(tracing_layer())
        .with(retry_layer(policy))
        .build();

    Ok(ReqwestMiddlewareClient::from(middleware))
}
