//! [`SttResponse`]: what an STT backend returns.

use elide_core::backend::{BackendResponse, Units};

use crate::modality::TranscriptSegment;

/// One per-call STT response from an STT backend.
///
/// Wraps the [`TranscriptSegment`]s the backend produced in backend order
/// (typically source order). These are the core transcription type, so an
/// enricher folds them into a [`Transcription`] and onto the call's
/// artifacts without any remapping.
///
/// [`Transcription`]: crate::modality::Transcription
#[derive(Debug, Clone, Default)]
pub struct SttResponse {
    /// Segments predicted for the request, in backend order.
    pub segments: Vec<TranscriptSegment>,
}

impl SttResponse {
    /// Construct a response from segments.
    #[must_use]
    pub fn new(segments: Vec<TranscriptSegment>) -> Self {
        Self { segments }
    }
}

impl BackendResponse for SttResponse {
    /// No billing units: STT bills per second of processed audio, but the
    /// transcript carries only segment spans, which under-report duration when a
    /// clip has trailing silence or an empty transcript. Reporting a wrong number
    /// is worse than none, so this stays empty until a backend surfaces the
    /// provider's own reported audio duration.
    fn units(&self) -> Units {
        Units::none()
    }

    fn output_count(&self) -> Option<u64> {
        Some(self.segments.len() as u64)
    }
}
