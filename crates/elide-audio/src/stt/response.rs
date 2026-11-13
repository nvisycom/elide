//! [`SttResponse`]: what an STT backend returns.

use elide_core::backend::Response;
use elide_core::entity::audit::ModelId;

use crate::modality::TranscriptSegment;

/// The transcript an STT backend returns for one clip, its segments, and the
/// model that transcribed it.
///
/// The [`TranscriptSegment`]s come in backend order (typically source order).
/// They are the core transcription type, so an enricher folds them into a
/// [`Transcription`] and onto the call's artifacts without any remapping.
///
/// [`Transcription`]: crate::modality::Transcription
#[derive(Debug, Clone, Response)]
pub struct SttResponse {
    /// The model that transcribed the clip.
    #[backend(model)]
    pub model: ModelId,
    /// Segments predicted for the request, in backend order.
    pub segments: Vec<TranscriptSegment>,
}

impl SttResponse {
    /// The `segments` `model` transcribed.
    #[must_use]
    pub fn new(model: ModelId, segments: Vec<TranscriptSegment>) -> Self {
        Self { model, segments }
    }
}
