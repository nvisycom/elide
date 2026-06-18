//! HTTP client configuration.

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Default request timeout (2 minutes).
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);
/// Default connection timeout (10 seconds).
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Default maximum number of retries.
const DEFAULT_MAX_RETRIES: u32 = 3;
/// Default keep-alive idle timeout (90 seconds).
const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(90);

/// Configuration for the shared HTTP client.
///
/// Durations accept any human-readable form (`"120s"`, `"2min"`,
/// `"500ms"`) via [`humantime_serde`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HttpConfig {
    /// Maximum number of retries for transient failures (default: 3).
    #[serde(default = "default_max_retries", alias = "max_retries")]
    pub max_retries: u32,
    /// Per-request timeout (default: 2m).
    #[serde(
        default = "default_timeout",
        with = "humantime_serde",
        alias = "timeout"
    )]
    pub timeout: Duration,
    /// TCP connection timeout (default: 10s).
    #[serde(
        default = "default_connect_timeout",
        with = "humantime_serde",
        alias = "connect_timeout"
    )]
    pub connect_timeout: Duration,
    /// Keep-alive pool idle timeout (default: 90s).
    #[serde(
        default = "default_idle_timeout",
        with = "humantime_serde",
        alias = "idle_timeout",
        alias = "pool_idle_timeout"
    )]
    pub idle_timeout: Duration,
}

fn default_max_retries() -> u32 {
    DEFAULT_MAX_RETRIES
}
fn default_timeout() -> Duration {
    DEFAULT_TIMEOUT
}
fn default_connect_timeout() -> Duration {
    DEFAULT_CONNECT_TIMEOUT
}
fn default_idle_timeout() -> Duration {
    DEFAULT_IDLE_TIMEOUT
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            max_retries: DEFAULT_MAX_RETRIES,
            timeout: DEFAULT_TIMEOUT,
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            idle_timeout: DEFAULT_IDLE_TIMEOUT,
        }
    }
}
