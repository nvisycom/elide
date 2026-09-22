//! Speech-to-text: transcribe an audio clip and enrich the call with a
//! [`Transcription`](crate::modality::Transcription).
//!
//! The [`SttBackend`] trait covers every STT engine (hosted transcription APIs,
//! local models, the in-process test stub); the [`SttEnricher`] drives a backend
//! per call and stamps the recognized
//! [`Transcription`](crate::modality::Transcription) onto the audio so a
//! recognizer can read it. The no-op `MockBackend` is behind `test-utils`.

mod backend;
mod enricher;

#[cfg(any(test, feature = "test-utils"))]
#[cfg_attr(docsrs, doc(cfg(feature = "test-utils")))]
pub use self::backend::MockBackend;
pub use self::backend::{SttBackend, SttRequest, SttResponse};
pub use self::enricher::{SttEnricher, SttEnricherBuilder};
