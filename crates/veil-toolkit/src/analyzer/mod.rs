//! The [`Analyzer`] — the "find" engine.
//!
//! Wraps recognizers and a deduplication pipeline into one
//! Presidio-style entry point. Recognizers and
//! [`Layer`](crate::deduplication::Layer)s are added with the `with_*`
//! builders; [`analyze`](Analyzer::analyze) runs the recognizers, then
//! the layers, returning a clean entity set.

mod dyn_recognizer;
mod registry;

pub use self::registry::RecognizerRegistry;

use veil_core::Error;
use veil_core::entity::Entity;
use veil_core::modality::Modality;
use veil_core::recognition::{Recognizer, RecognizerInput};

use crate::deduplication::{Layer, LayerPipeline};

/// The find engine: recognizers + deduplication, in one call.
///
/// Generic over the [`Modality`] `M`. Recognizers are added with
/// [`with_recognizer`](Analyzer::with_recognizer) and deduplication
/// stages with [`with_layer`](Analyzer::with_layer), in the order they
/// should run. [`analyze`](Analyzer::analyze) runs detection, then every
/// layer, and returns the reconciled entities.
///
/// ```ignore
/// let entities = Analyzer::new()
///     .with_recognizer(us_phone)
///     .with_recognizer(ner)
///     .with_layer(FuseLayer::with_strategy(MaxConfidence))
///     .with_layer(ResolveLayer::new(HighestConfidence))
///     .with_layer(FilterLayer::new().with_threshold(ConfidenceThreshold::BASELINE))
///     .analyze(input)
///     .await?;
/// ```
pub struct Analyzer<M: Modality> {
    recognizers: RecognizerRegistry,
    pipeline: LayerPipeline<M>,
}

impl<M: Modality> Analyzer<M> {
    /// An analyzer with no recognizers and no layers.
    pub fn new() -> Self {
        Self {
            recognizers: RecognizerRegistry::new(),
            pipeline: LayerPipeline::new(),
        }
    }

    /// Add a recognizer for modality `M`.
    #[must_use]
    pub fn with_recognizer<R: Recognizer<M> + 'static>(mut self, recognizer: R) -> Self {
        self.recognizers = self.recognizers.with_recognizer::<M, _>(recognizer);
        self
    }

    /// Append a deduplication layer. Layers run in the order added,
    /// after detection.
    #[must_use]
    pub fn with_layer<L: Layer<M> + 'static>(mut self, layer: L) -> Self {
        self.pipeline.push(layer);
        self
    }

    /// Find entities in `input`: run every recognizer, then every layer.
    ///
    /// Returns the reconciled entity set. Propagates the first
    /// recognizer error.
    pub async fn analyze(&self, input: RecognizerInput<M>) -> Result<Vec<Entity<M>>, Error> {
        let entities = self.recognizers.run(input).await?;
        Ok(self.pipeline.run(entities))
    }
}

impl<M: Modality> Default for Analyzer<M> {
    fn default() -> Self {
        Self::new()
    }
}
