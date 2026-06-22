//! HTTP client factory for the rig backend (reqwest + retry + tracing
//! middleware).
//!
//! [`build_http_client`] builds a [`ClientWithMiddleware`] from an
//! [`HttpConfig`] with exponential-backoff retry and OpenTelemetry tracing
//! layers pre-installed; the [`RigBackend`] hands the returned client to its
//! rig provider client at construction.
//!
//! [`ClientWithMiddleware`]: reqwest_middleware::ClientWithMiddleware
//! [`RigBackend`]: super::RigBackend

mod config;
mod middleware;

use elide_core::{Error, ErrorKind, Result};
use reqwest_middleware::reqwest::Client;
use reqwest_middleware::{ClientBuilder, ClientWithMiddleware};

pub use self::config::HttpConfig;
use self::middleware::{backoff_policy, retry_layer, tracing_layer};

const TARGET: &str = "elide_llm::http";

/// Build a [`ClientWithMiddleware`] from the given configuration.
///
/// The returned client has exponential-backoff retry and OpenTelemetry
/// tracing middleware pre-installed; the rig backend wires it into the
/// provider's rig client.
///
/// # Errors
///
/// Returns an error if the underlying `reqwest::Client` cannot be
/// built (e.g. TLS backend initialisation failure).
pub fn build_http_client(config: &HttpConfig) -> Result<ClientWithMiddleware> {
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
                ErrorKind::Validation,
                format!("failed to build HTTP client: {e}"),
            )
        })?;

    Ok(ClientBuilder::new(client)
        .with(tracing_layer())
        .with(retry_layer(policy))
        .build())
}
