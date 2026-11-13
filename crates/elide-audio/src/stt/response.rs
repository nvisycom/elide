//! [`SttResponse`]: what an STT backend returns.

use elide_core::backend::BackendResponse;

use crate::modality::TranscriptSegment;

/// The transcript an STT backend returns for one clip: its segments.
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

impl BackendResponse for SttResponse {}
