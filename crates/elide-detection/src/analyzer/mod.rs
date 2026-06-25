//! The [`Analyzer`]: the "find" engine.
//!
//! Wraps enrichers, recognizers, and a deduplication pipeline into one
//! Presidio-style entry point. Enrichers, recognizers, and [`Layer`]s are
//! added with the `with_*` builders; [`analyze`] runs three phases in
//! order: enrich (sequential), recognize (concurrent), reduce (the
//! layers), returning a clean entity set.
//!
//! [`Layer`]: crate::deduplication::Layer
//! [`analyze`]: Analyzer::analyze

mod dyn_enricher;
mod dyn_recognizer;

use std::sync::Arc;

use elide_core::Result;
use elide_core::entity::Entity;
use elide_core::modality::{Modality, ModalityLocation, StreamDataReader};
use elide_core::recognition::annotation::Exclusion;
use elide_core::recognition::{Enricher, Recognizer, RecognizerContext, Scope};
use futures::future;

use self::dyn_enricher::DynEnricher;
use self::dyn_recognizer::DynRecognizer;
use crate::deduplication::Layer;

/// The find engine: enrichers, recognizers, and deduplication, in one
/// call.
///
/// Generic over the [`Modality`] `M`. Enrichers, recognizers, and
/// deduplication layers are added with [`with_enricher`],
/// [`with_recognizer`], and [`with_layer`], each in the order it should
/// run. [`analyze`] runs the three phases and returns the reconciled
/// entities.
///
/// ```ignore
/// let entities = Analyzer::new()
///     .with_enricher(lingua)
///     .with_recognizer(us_phone)
///     .with_recognizer(ner)
///     .with_layer(FuseLayer::new(MaxConfidence))
///     .with_layer(ResolveLayer::new(HighestConfidence))
///     .with_layer(FilterLayer::new().with_threshold(ConfidenceThreshold::BASELINE))
///     .analyze(data, &Scope::new())
///     .await?;
/// ```
///
/// [`with_enricher`]: Analyzer::with_enricher
/// [`with_recognizer`]: Analyzer::with_recognizer
/// [`with_layer`]: Analyzer::with_layer
/// [`analyze`]: Analyzer::analyze
pub struct Analyzer<M: Modality> {
    enrichers: Vec<Arc<dyn DynEnricher<M>>>,
    recognizers: Vec<Arc<dyn DynRecognizer<M>>>,
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

    /// Add an enricher. Enrichers run in the order added, sequentially,
    /// before any recognizer (so a recognizer sees what they wrote onto
    /// the input).
    #[must_use]
    pub fn with_enricher<E: Enricher<M> + 'static>(mut self, enricher: E) -> Self {
        self.enrichers.push(Arc::new(enricher));
        self
    }

    /// Add a recognizer. Recognizers run concurrently during the
    /// recognition phase.
    #[must_use]
    pub fn with_recognizer<R: Recognizer<M> + 'static>(mut self, recognizer: R) -> Self {
        self.recognizers.push(Arc::new(recognizer));
        self
    }

    /// Append a deduplication layer. Layers run in the order added,
    /// after detection.
    #[must_use]
    pub fn with_layer<L: Layer<M> + 'static>(mut self, layer: L) -> Self {
        self.layers.push(Arc::new(layer));
        self
    }

    /// Run the three analysis phases over one payload: every enricher
    /// (sequentially) to fill in the working context, then every
    /// recognizer (concurrently), then every deduplication layer.
    ///
    /// `scope` is the caller's asserted scope; a fresh working
    /// [`RecognizerContext`] is built per payload, borrowing the scope and
    /// owning that payload's artifacts. The shared core behind [`analyze`]
    /// and [`analyze_stream`].
    ///
    /// [`analyze`]: Self::analyze
    /// [`analyze_stream`]: Self::analyze_stream
    async fn analyze_core(
        &self,
        data: M::Data,
        ctx: &mut RecognizerContext<'_, M>,
    ) -> Result<Vec<Entity<M>>> {
        for enricher in &self.enrichers {
            enricher.enrich_boxed(&data, ctx).await?;
        }
        let mut entities = self.recognize(&data, ctx).await?;
        ctx.stamp_languages(&mut entities);
        let reduced = self.reduce(entities);
        Ok(Self::apply_exclusions(reduced, ctx.exclusions()))
    }

    /// Run every deduplication layer in order over `entities`, threading
    /// each layer's kept output into the next and returning the survivors.
    fn reduce(&self, mut entities: Vec<Entity<M>>) -> Vec<Entity<M>> {
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
            "deduplication complete"
        );
        entities
    }

    /// Drop every entity whose location overlaps a caller [`Exclusion`].
    ///
    /// Runs after deduplication so it culls the reconciled set, not
    /// per-recognizer duplicates. A no-op when no exclusions are asserted.
    ///
    /// [`Exclusion`]: elide_core::recognition::annotation::Exclusion
    fn apply_exclusions(entities: Vec<Entity<M>>, exclusions: &[Exclusion<M>]) -> Vec<Entity<M>> {
        if exclusions.is_empty() {
            return entities;
        }
        entities
            .into_iter()
            .filter(|entity| {
                !exclusions
                    .iter()
                    .any(|exclusion| entity.location.overlaps(&exclusion.location))
            })
            .collect()
    }

    /// Analyze a single in-memory payload in the given scope.
    ///
    /// Runs the full analysis pipeline over `data`, with `scope` supplying
    /// the caller's assertions (languages, jurisdictions, labels,
    /// inclusions, exclusions).
    /// Use [`analyze_stream`] for an I/O-backed source that yields many
    /// chunks.
    ///
    /// [`analyze_stream`]: Self::analyze_stream
    pub async fn analyze(&self, data: M::Data, scope: &Scope<M>) -> Result<Vec<Entity<M>>> {
        let mut ctx = RecognizerContext::new(scope);
        self.analyze_core(data, &mut ctx).await
    }

    /// Analyze a streamed source end to end, returning entities in the
    /// source's own coordinate system.
    ///
    /// Drives `source` chunk by chunk: for each [`Chunk`], runs the full
    /// analysis pipeline over its payload in a fresh context (carrying the
    /// `scope` plus the chunk's own context hints), then [`lift`]s every
    /// entity from chunk-local to source coordinates, dropping any whose
    /// location has no source pre-image. The result aggregates every
    /// chunk's lifted entities.
    ///
    /// This is the [`analyze`] counterpart for I/O-backed sources (a
    /// decoded codec document, say): the caller never sees a chunk or a
    /// recognizer-local coordinate. Deduplication runs per chunk, the
    /// way [`analyze`] reduces a single payload.
    ///
    /// Returns the first enricher, recognizer, or read error.
    ///
    /// [`Chunk`]: elide_core::modality::Chunk
    /// [`analyze`]: Self::analyze
    /// [`lift`]: elide_core::modality::StreamDataReader::lift
    pub async fn analyze_stream<S>(
        &self,
        source: &mut S,
        scope: &Scope<M>,
    ) -> Result<Vec<Entity<M>>>
    where
        S: StreamDataReader<M>,
    {
        let mut out = Vec::new();
        while let Some(chunk) = source.read_next().await? {
            let mut ctx = RecognizerContext::new(scope).with_context_hints(chunk.hints.clone());
            let entities = self.analyze_core(chunk.data.clone(), &mut ctx).await?;
            out.extend(
                entities
                    .into_iter()
                    .filter_map(|entity| source.lift(&chunk, entity)),
            );
        }
        Ok(out)
    }

    /// Run every recognizer over `data` concurrently and collect their
    /// entities. The first error is returned (fail-fast).
    ///
    /// Recognizers borrow `data` and `ctx`, so they are joined in place
    /// rather than spawned onto the runtime.
    async fn recognize(
        &self,
        data: &M::Data,
        ctx: &RecognizerContext<'_, M>,
    ) -> Result<Vec<Entity<M>>> {
        if self.recognizers.is_empty() {
            return Ok(Vec::new());
        }

        let futures = self
            .recognizers
            .iter()
            .map(|recognizer| recognizer.recognize_boxed(data, ctx));
        let mut entities = Vec::new();
        for found in future::join_all(futures).await {
            entities.extend(found?);
        }
        Ok(entities)
    }
}

impl<M: Modality> Default for Analyzer<M> {
    fn default() -> Self {
        Self::new()
    }
}
