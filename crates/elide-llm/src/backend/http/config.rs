//! HTTP client configuration.

use std::time::Duration;

/// Default request timeout (2 minutes).
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);
/// Default connection timeout (10 seconds).
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Default maximum number of retries.
const DEFAULT_MAX_RETRIES: u32 = 3;
/// Default keep-alive idle timeout (90 seconds).
const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(90);

/// Configuration for the rig backend's HTTP client.
#[derive(Debug, Clone)]
pub struct HttpConfig {
    /// Maximum number of retries for transient failures (default: 3).
    pub max_retries: u32,
    /// Per-request timeout (default: 2m).
    pub timeout: Duration,
    /// TCP connection timeout (default: 10s).
    pub connect_timeout: Duration,
    /// Keep-alive pool idle timeout (default: 90s).
    pub idle_timeout: Duration,
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
