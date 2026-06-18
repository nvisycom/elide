//! The crate-private [`LayerPipeline`] — an ordered sequence of
//! [`Layer`]s, owned and run by an [`Analyzer`].
//!
//! [`Analyzer`]: crate::Analyzer

use elide_core::entity::Entity;
use elide_core::modality::Modality;

use super::Layer;

/// An ordered sequence of deduplication [`Layer`]s, run back to back.
///
/// Each layer's kept output feeds the next; dropped entities are
/// accumulated and reported via tracing. Holds boxed layers so a
/// pipeline can mix concretely-typed stages in one list.
///
/// Crate-private: consumers compose layers via [`Analyzer::with_layer`],
/// which forwards here. The pipeline itself is never named in the public
/// API.
///
/// [`Analyzer::with_layer`]: crate::Analyzer::with_layer
pub(crate) struct LayerPipeline<M: Modality> {
    layers: Vec<Box<dyn Layer<M>>>,
}

impl<M: Modality> LayerPipeline<M> {
    /// An empty pipeline.
    pub(crate) fn new() -> Self {
        Self { layers: Vec::new() }
    }

    /// Append a layer.
    pub(crate) fn push<L: Layer<M> + 'static>(&mut self, layer: L) {
        self.layers.push(Box::new(layer));
    }

    /// Run every layer in order over `entities`, returning the survivors.
    pub(crate) fn run(&self, mut entities: Vec<Entity<M>>) -> Vec<Entity<M>> {
        let before = entities.len();
        let mut dropped = 0usize;
        for layer in &self.layers {
            let output = layer.apply(entities);
            dropped += output.dropped.len();
            entities = output.kept;
        }
        tracing::debug!(
            modality = M::NAME,
            before,
            after = entities.len(),
            dropped,
            "deduplication pipeline complete"
        );
        entities
    }
}
