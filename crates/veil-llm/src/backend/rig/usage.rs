//! Cumulative token-usage tracking across LLM requests.

use std::sync::atomic::{AtomicU64, Ordering};

use derive_more::AddAssign;
use rig::completion::Usage;

/// Thread-safe accumulator for LLM token usage.
///
/// Uses lock-free atomics instead of a mutex — all operations are
/// wait-free counter increments/loads. Each [`RigBackend`] owns one
/// tracker; callers snapshot it to inspect costs.
///
/// [`RigBackend`]: super::RigBackend
pub struct UsageTracker {
    input_tokens: AtomicU64,
    output_tokens: AtomicU64,
    requests: AtomicU64,
    retries: AtomicU64,
}

/// Point-in-time snapshot of accumulated usage counters.
///
/// Field-wise `AddAssign` is derived so pipelines holding multiple
/// backends can sum their snapshots into a single aggregate.
#[derive(Debug, Default, Clone, AddAssign)]
pub struct UsageStats {
    /// Cumulative input (prompt) tokens billed across every request.
    pub total_input_tokens: u64,
    /// Cumulative output (completion) tokens billed across every request.
    pub total_output_tokens: u64,
    /// Number of completed LLM requests.
    pub total_requests: u64,
    /// Number of retry attempts triggered by transient failures.
    pub total_retries: u64,
}

impl UsageTracker {
    /// Build a tracker with every counter at zero.
    pub fn new() -> Self {
        Self {
            input_tokens: AtomicU64::new(0),
            output_tokens: AtomicU64::new(0),
            requests: AtomicU64::new(0),
            retries: AtomicU64::new(0),
        }
    }

    /// Record a single LLM request's token usage and retry count.
    pub fn record(&self, usage: &Usage, retries: u32) {
        self.input_tokens
            .fetch_add(usage.input_tokens, Ordering::Relaxed);
        self.output_tokens
            .fetch_add(usage.output_tokens, Ordering::Relaxed);
        self.requests.fetch_add(1, Ordering::Relaxed);
        self.retries
            .fetch_add(u64::from(retries), Ordering::Relaxed);
    }

    /// Snapshot the current counters without resetting them.
    pub fn snapshot(&self) -> UsageStats {
        UsageStats {
            total_input_tokens: self.input_tokens.load(Ordering::Relaxed),
            total_output_tokens: self.output_tokens.load(Ordering::Relaxed),
            total_requests: self.requests.load(Ordering::Relaxed),
            total_retries: self.retries.load(Ordering::Relaxed),
        }
    }

    /// Reset all counters to zero.
    pub fn reset(&self) {
        self.input_tokens.store(0, Ordering::Relaxed);
        self.output_tokens.store(0, Ordering::Relaxed);
        self.requests.store(0, Ordering::Relaxed);
        self.retries.store(0, Ordering::Relaxed);
    }
}

impl Default for UsageTracker {
    fn default() -> Self {
        Self::new()
    }
}
