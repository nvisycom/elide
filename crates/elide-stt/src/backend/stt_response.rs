//! [`SttResponse`]: what an [`SttBackend`] returns.
//!
//! [`SttBackend`]: super::SttBackend

use super::transcribed_segment::TranscribedSegment;

/// One per-call STT response from an [`SttBackend`].
///
/// Wraps the segments the backend produced in backend order (typically
/// source order). The recognition layer treats each segment as a chunk of
/// transcript text addressable by its `[start_ms, end_ms)` interval.
///
/// [`SttBackend`]: super::SttBackend
#[derive(Debug, Clone, Default)]
pub struct SttResponse {
    /// Segments predicted for the request, in backend order.
    pub segments: Vec<TranscribedSegment>,
}

impl SttResponse {
    /// Construct a response from segments.
    #[must_use]
    pub fn new(segments: Vec<TranscribedSegment>) -> Self {
        Self { segments }
    }
}
