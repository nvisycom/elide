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

use elide_core::entity::Entity;
use elide_core::modality::{Modality, StreamDataReader};
use elide_core::recognition::{Enricher, Recognizer, RecognizerContext};
use elide_core::{Error, ErrorKind, Result};
use tokio::task::JoinSet;

use self::dyn_enricher::DynEnricher;
use self::dyn_recognizer::DynRecognizer;
use crate::deduplication::{Layer, LayerPipeline};

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
///     .analyze(data, &RecognizerContext::new())
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
    pipeline: LayerPipeline<M>,
}

impl<M: Modality> Analyzer<M> {
    /// An analyzer with no enrichers, recognizers, or layers.
    pub fn new() -> Self {
        Self {
            enrichers: Vec::new(),
            recognizers: Vec::new(),
            pipeline: LayerPipeline::new(),
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
        self.pipeline.push(layer);
        self
    }

    /// Run the three analysis phases over one payload: every enricher
    /// (sequentially) to fill in per-call context, then every recognizer
    /// (concurrently), then every deduplication layer.
    ///
    /// `base` carries the caller's asserted context; a fresh working copy
    /// is forked per call so enrichers stamp their own artifacts. The
    /// shared core behind [`analyze`] and [`analyze_stream`].
    ///
    /// [`analyze`]: Self::analyze
    /// [`analyze_stream`]: Self::analyze_stream
    async fn analyze_core(
        &self,
        data: M::Data,
        base: &RecognizerContext<M>,
    ) -> Result<Vec<Entity<M>>> {
        let mut ctx = base.fork_assertions();
        for enricher in &self.enrichers {
            enricher.enrich_boxed(&data, &mut ctx).await?;
        }
        let entities = self.recognize(data, ctx).await?;
        Ok(self.pipeline.run(entities))
    }

    /// Analyze a single in-memory payload in the given context.
    ///
    /// Runs the full analysis pipeline over `data`, with `ctx` supplying
    /// the per-call assertions (languages, jurisdictions, labels, hints).
    /// Use [`analyze_stream`] for an I/O-backed source that yields many
    /// chunks.
    ///
    /// [`analyze_stream`]: Self::analyze_stream
    pub async fn analyze(
        &self,
        data: M::Data,
        ctx: &RecognizerContext<M>,
    ) -> Result<Vec<Entity<M>>> {
        self.analyze_core(data, ctx).await
    }

    /// Analyze a streamed source end to end, returning entities in the
    /// source's own coordinate system.
    ///
    /// Drives `source` chunk by chunk: for each [`Chunk`], runs the full
    /// analysis pipeline over its payload in `ctx` (extended with the
    /// chunk's own context hints), then [`lift`]s every entity from
    /// chunk-local to source coordinates, dropping any whose location has
    /// no source pre-image. The result aggregates every chunk's lifted
    /// entities.
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
        ctx: &RecognizerContext<M>,
    ) -> Result<Vec<Entity<M>>>
    where
        S: StreamDataReader<M>,
    {
        let mut out = Vec::new();
        while let Some(chunk) = source.read_next().await? {
            let chunk_ctx = ctx
                .fork_assertions()
                .with_context_hints(chunk.hints.clone());
            let entities = self.analyze_core(chunk.data.clone(), &chunk_ctx).await?;
            out.extend(
                entities
                    .into_iter()
                    .filter_map(|entity| source.lift(&chunk, entity)),
            );
        }
        Ok(out)
    }

    /// Run every recognizer over `data` concurrently and collect their
    /// entities. The first error aborts the rest and is returned
    /// (fail-fast).
    async fn recognize(&self, data: M::Data, ctx: RecognizerContext<M>) -> Result<Vec<Entity<M>>> {
        if self.recognizers.is_empty() {
            return Ok(Vec::new());
        }

        let data = Arc::new(data);
        let ctx = Arc::new(ctx);
        let mut set: JoinSet<Result<Vec<Entity<M>>>> = JoinSet::new();
        for recognizer in &self.recognizers {
            let recognizer = Arc::clone(recognizer);
            let data = Arc::clone(&data);
            let ctx = Arc::clone(&ctx);
            set.spawn(async move { recognizer.recognize_boxed(&data, &ctx).await });
        }

        let mut entities = Vec::new();
        while let Some(joined) = set.join_next().await {
            match joined {
                Ok(Ok(found)) => entities.extend(found),
                Ok(Err(error)) => {
                    set.abort_all();
                    return Err(error);
                }
                Err(join) => {
                    set.abort_all();
                    return Err(Error::new(ErrorKind::Recognition, join));
                }
            }
        }
        Ok(entities)
    }
}

impl<M: Modality> Default for Analyzer<M> {
    fn default() -> Self {
        Self::new()
    }
}
