//! NLP enrichers: pre-recognition passes that annotate the input.
//!
//! Each runs ahead of the recognizers and context enhancer, annotating the
//! input for the stages that follow.
//!
//! A pipeline registers an enricher ahead of its recognizers; the enricher
//! resolves some property onto the call, and downstream stages read it
//! back from the input. The enrichment-result types live in `elide-core`.
//!
//! Language detection is the first such enricher: [`LinguaEnricher`],
//! backed by the [`lingua`] crate, detects the document's language(s).
//! Other enrichers (tokenization, sentence segmentation, …) belong here
//! too.
//!
//! NER itself is a separate trait-driven path ([`NerBackend`] +
//! [`NerRecognizer`]); recognition backends plug in there, not here.
//!
//! [`lingua`]: https://crates.io/crates/lingua
//! [`NerBackend`]: crate::backend::NerBackend
//! [`NerRecognizer`]: crate::NerRecognizer

mod lingua_detector;
mod lingua_enricher;

pub use self::lingua_enricher::LinguaEnricher;
