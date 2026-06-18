//! Language detection.
//!
//! [`LinguaEnricher`] is a language-detection enricher backed by the
//! [`lingua`] crate. A pipeline registers one ahead of its recognizers to
//! resolve the document's language onto the call; recognizers and the
//! context enhancer then read the languages back from the input. The
//! language-result types it produces live in `elide-core`.
//!
//! NER lives in a separate trait-driven path ([`NerBackend`] +
//! [`NerRecognizer`]); detection backends plug in there, not here.
//!
//! [`lingua`]: https://crates.io/crates/lingua
//! [`NerBackend`]: crate::backend::NerBackend
//! [`NerRecognizer`]: crate::NerRecognizer

mod lingua_detector;
mod lingua_enricher;

pub use self::lingua_detector::LinguaDetector;
pub use self::lingua_enricher::LinguaEnricher;
