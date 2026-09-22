//! Enrichment: pre-recognition passes that annotate the input.
//!
//! Each [`Enricher`] runs ahead of the recognizers, resolving some property
//! onto the call that downstream stages read back, the same seam, whether
//! it detects a language, transcribes audio, or OCRs an image. Each shipped
//! enricher sits behind a feature: `lingua` (language detection), `stt`
//! (speech-to-text + the transcript enricher), and `ocr` (OCR + the
//! recognized-text enricher).
//!
//! [`Enricher`]: elide_core::enrichment::Enricher

/// Speech-to-text backends and the enricher that runs text recognizers
/// over the transcript.
#[cfg(feature = "stt")]
#[cfg_attr(docsrs, doc(cfg(feature = "stt")))]
#[doc(inline)]
pub use elide_audio::stt;
#[doc(inline)]
pub use elide_core::enrichment::{Enricher, Enrichment};
/// OCR backends and the enricher that runs text recognizers over the
/// recognized image text.
#[cfg(feature = "ocr")]
#[cfg_attr(docsrs, doc(cfg(feature = "ocr")))]
#[doc(inline)]
pub use elide_image::ocr;
/// Language detection for language-aware recognizers and policies.
#[cfg(feature = "lingua")]
#[cfg_attr(docsrs, doc(cfg(feature = "lingua")))]
#[doc(inline)]
pub use elide_lingua as lingua;
