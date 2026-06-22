//! Time-interval primitives for stream-addressed media.

use std::cmp::Ordering;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Microseconds per millisecond.
const MICROS_PER_MILLI: u64 = 1_000;
/// Microseconds per second.
const MICROS_PER_SECOND: u64 = 1_000_000;

/// A half-open `[start, end)` interval within a stream, measured in
/// microseconds from the start of the stream.
///
/// The coordinate a time-addressed medium (audio, video) uses to locate a
/// region: a transcribed segment, a redacted span. Microsecond precision
/// is finer than both word-level speech timings and per-sample audio
/// resolution, so a span never loses precision being carried as a
/// `TimeSpan`; the endpoints are non-negative offsets by construction.
///
/// Half-open like a byte range: `[start, end)`, so two intervals that
/// merely touch (`a.end == b.start`) do not [`overlap`](Self::overlaps).
///
/// [`from_millis`](Self::from_millis) and [`as_millis`](Self::start_millis)
/// bridge the millisecond-based APIs that surround it (audio durations,
/// provider timings reported in ms).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TimeSpan {
    /// Microseconds from the start of the stream where the interval begins.
    start_us: u64,
    /// Microseconds from the start of the stream where the interval ends
    /// (exclusive).
    end_us: u64,
}

impl TimeSpan {
    /// An interval over `[start_us, end_us)` in microseconds.
    ///
    /// `end_us` is clamped up to `start_us` when it would precede it, so an
    /// interval is never negative-length.
    #[must_use]
    pub fn new(start_us: u64, end_us: u64) -> Self {
        Self {
            start_us,
            end_us: end_us.max(start_us),
        }
    }

    /// An interval over `[start_ms, end_ms)` in milliseconds.
    #[must_use]
    pub fn from_millis(start_ms: u64, end_ms: u64) -> Self {
        Self::new(start_ms * MICROS_PER_MILLI, end_ms * MICROS_PER_MILLI)
    }

    /// Start offset in microseconds.
    #[must_use]
    pub fn start_micros(&self) -> u64 {
        self.start_us
    }

    /// End offset (exclusive) in microseconds.
    #[must_use]
    pub fn end_micros(&self) -> u64 {
        self.end_us
    }

    /// Start offset truncated to whole milliseconds.
    #[must_use]
    pub fn start_millis(&self) -> u64 {
        self.start_us / MICROS_PER_MILLI
    }

    /// End offset (exclusive) truncated to whole milliseconds.
    #[must_use]
    pub fn end_millis(&self) -> u64 {
        self.end_us / MICROS_PER_MILLI
    }

    /// Length of the interval in microseconds (`end - start`).
    #[must_use]
    pub fn duration_micros(&self) -> u64 {
        self.end_us - self.start_us
    }

    /// Length of the interval truncated to whole milliseconds.
    #[must_use]
    pub fn duration_millis(&self) -> u64 {
        self.duration_micros() / MICROS_PER_MILLI
    }

    /// Whether the interval is empty (zero length).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.start_us == self.end_us
    }

    /// Whether this interval overlaps `other`.
    ///
    /// Half-open intersection: touching-but-disjoint intervals (one ending
    /// exactly where the other starts) do not overlap.
    #[must_use]
    pub fn overlaps(&self, other: &Self) -> bool {
        self.start_us < other.end_us && other.start_us < self.end_us
    }

    /// Whether `micros` falls within `[start, end)`.
    #[must_use]
    pub fn contains_micros(&self, micros: u64) -> bool {
        self.start_us <= micros && micros < self.end_us
    }

    /// Order by length: the longer interval is [`Greater`].
    ///
    /// [`Greater`]: std::cmp::Ordering::Greater
    #[must_use]
    pub fn duration_cmp(&self, other: &Self) -> Ordering {
        self.duration_micros().cmp(&other.duration_micros())
    }

    /// Order by position in the stream: by start, then by end.
    #[must_use]
    pub fn position_cmp(&self, other: &Self) -> Ordering {
        self.start_us
            .cmp(&other.start_us)
            .then(self.end_us.cmp(&other.end_us))
    }
}

impl std::fmt::Display for TimeSpan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let secs = |us: u64| us as f64 / MICROS_PER_SECOND as f64;
        write!(
            f,
            "[{:.3}s, {:.3}s)",
            secs(self.start_us),
            secs(self.end_us)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_millis_converts_to_micros() {
        let span = TimeSpan::from_millis(100, 200);
        assert_eq!(span.start_micros(), 100_000);
        assert_eq!(span.end_micros(), 200_000);
        assert_eq!(span.duration_millis(), 100);
    }

    #[test]
    fn new_clamps_inverted_interval_to_zero_length() {
        let span = TimeSpan::new(500, 200);
        assert_eq!(span.start_micros(), 500);
        assert_eq!(span.end_micros(), 500);
        assert!(span.is_empty());
    }

    #[test]
    fn overlaps_is_half_open() {
        let a = TimeSpan::new(0, 1_000);
        let b = TimeSpan::new(500, 1_500);
        assert!(a.overlaps(&b));
        // Touching but disjoint: a ends exactly where c starts.
        let c = TimeSpan::new(1_000, 2_000);
        assert!(!a.overlaps(&c));
    }

    #[test]
    fn contains_micros_is_half_open() {
        let span = TimeSpan::new(100, 200);
        assert!(span.contains_micros(100));
        assert!(span.contains_micros(199));
        assert!(!span.contains_micros(200));
    }

    #[test]
    fn duration_cmp_orders_by_length() {
        let short = TimeSpan::new(0, 200);
        let long = TimeSpan::new(0, 1_000);
        assert_eq!(short.duration_cmp(&long), Ordering::Less);
    }

    #[test]
    fn position_cmp_orders_by_start_then_end() {
        let early = TimeSpan::new(0, 5_000);
        let late = TimeSpan::new(1_000, 2_000);
        assert_eq!(early.position_cmp(&late), Ordering::Less);
        // Same start, shorter end sorts first.
        let a = TimeSpan::new(1_000, 1_500);
        let b = TimeSpan::new(1_000, 3_000);
        assert_eq!(a.position_cmp(&b), Ordering::Less);
    }
}
