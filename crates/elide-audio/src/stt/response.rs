//! [`SttResponse`]: what an [`SttBackend`] returns.
//!
//! [`SttBackend`]: super::SttBackend

use elide_core::backend::{BackendResponse, Meter, Units};

use crate::modality::TranscriptSegment;

/// One per-call STT response from an [`SttBackend`].
///
/// Wraps the [`TranscriptSegment`]s the backend produced in backend order
/// (typically source order). These are the core transcription type, so an
/// enricher folds them into a [`Transcription`] and onto the call's
/// artifacts without any remapping.
///
/// [`SttBackend`]: super::SttBackend
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
    /// STT bills per second of audio: the transcribed extent, taken as the
    /// latest segment end. Empty when no segment carries timing.
    fn units(&self) -> Units {
        let end_micros = self
            .segments
            .iter()
            .map(|segment| segment.span.end_micros())
            .max();
        match end_micros {
            Some(micros) => Units::from(Meter::Seconds(micros as f64 / 1_000_000.0)),
            None => Units::none(),
        }
    }

    fn output_count(&self) -> Option<u64> {
        Some(self.segments.len() as u64)
    }
}
