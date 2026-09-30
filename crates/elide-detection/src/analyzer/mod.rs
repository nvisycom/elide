//! The [`Analyzer`]: the "find" engine.
//!
//! Wraps enrichers, recognizers, and a deduplication pipeline into one
//! Presidio-style entry point. Enrichers, recognizers, and [`Layer`]s are added
//! with the `with_*` builders; [`analyze`] runs three phases in order: enrich
//! (sequential), recognize (concurrent), reduce (the layers), returning a clean
//! entity set.
//!
//! The module is split by concern: this file holds the [`Analyzer`] and its
//! builder; [`analysis`] the [`Analysis`] result type; [`engine`] the one
//! analysis core every entry funnels through; [`entry`] the public entry points
//! (single payload, one stream, many streams) as thin adapters onto that core.
//!
//! [`Layer`]: crate::layer::Layer
//! [`analyze`]: Analyzer::analyze

mod analysis;
mod engine;
mod entry;

use std::sync::Arc;

use elide_core::enrichment::Enricher;
use elide_core::modality::Modality;
use elide_core::recognition::Recognizer;

pub use self::analysis::Analysis;
use crate::layer::Layer;

/// The find engine: enrichers, recognizers, and deduplication, in one call.
///
/// Generic over the [`Modality`] `M`. Enrichers, recognizers, and deduplication
/// layers are added with [`with_enricher`], [`with_recognizer`], and
/// [`with_layer`], each in the order it should run. [`analyze`] runs the three
/// phases and returns the reconciled entities.
///
/// ```ignore
/// let entities = Analyzer::new()
///     .with_enricher(lingua)
///     .with_recognizer(us_phone)
///     .with_recognizer(ner)
///     .with_layer(ReconcileLayer::same_label(Merging::max()))
///     .with_layer(ReconcileLayer::cross_label(Structural::default()))
///     .with_layer(FilterLayer::new().with_threshold(ConfidenceThreshold::BASELINE))
///     .analyze(data, &Scope::new().with_catalog(LabelCatalog::with_builtins()))
///     .await?;
/// ```
///
/// [`with_enricher`]: Analyzer::with_enricher
/// [`with_recognizer`]: Analyzer::with_recognizer
/// [`with_layer`]: Analyzer::with_layer
/// [`analyze`]: Analyzer::analyze
pub struct Analyzer<M: Modality> {
    enrichers: Vec<Arc<dyn Enricher<M>>>,
    recognizers: Vec<Arc<dyn Recognizer<M>>>,
    layers: Vec<Arc<dyn Layer<M>>>,
}

impl<M: Modality> Analyzer<M> {
    /// An analyzer with no enrichers, recognizers, or layers.
    pub fn new() -> Self {
        Self {
            enrichers: Vec::new(),
            recognizers: Vec::new(),
            layers: Vec::new(),
        }
    }

    /// Add an enricher. Enrichers run in the order added, sequentially, before any
    /// recognizer (so a recognizer sees what they wrote onto the input).
    #[must_use]
    pub fn with_enricher<E: Enricher<M> + 'static>(mut self, enricher: E) -> Self {
        self.enrichers.push(Arc::new(enricher));
        self
    }

    /// Add a recognizer. Recognizers run concurrently during the recognition
    /// phase.
    #[must_use]
    pub fn with_recognizer<R: Recognizer<M> + 'static>(mut self, recognizer: R) -> Self {
        self.recognizers.push(Arc::new(recognizer));
        self
    }

    /// Append a deduplication layer. Layers run in the order added, after
    /// detection.
    #[must_use]
    pub fn with_layer<L: Layer<M> + 'static>(mut self, layer: L) -> Self {
        self.layers.push(Arc::new(layer));
        self
    }
}

impl<M: Modality> Default for Analyzer<M> {
    fn default() -> Self {
        Self::new()
    }
}
