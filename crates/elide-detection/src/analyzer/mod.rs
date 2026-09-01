//! The [`Analyzer`]: the "find" engine.
//!
//! Wraps enrichers, recognizers, and a deduplication pipeline into one
//! Presidio-style entry point. Enrichers, recognizers, and [`Layer`]s are
//! added with the `with_*` builders; [`analyze`] runs three phases in
//! order: enrich (sequential), recognize (concurrent), reduce (the
//! layers), returning a clean entity set.
//!
//! [`Layer`]: crate::layer::Layer
//! [`analyze`]: Analyzer::analyze

use std::sync::Arc;
#[cfg(feature = "usage")]
use std::time::{Duration, Instant};

use elide_core::Result;
use elide_core::entity::Entity;
use elide_core::modality::{Modality, ModalityLocation, StreamDataReader};
use elide_core::recognition::annotation::{Annotations, Exclusion};
use elide_core::recognition::{Enricher, Recognition, Recognizer, RecognizerContext, Scope};
#[cfg(feature = "usage")]
use elide_core::recognition::{RecognizerId, Usage};
use futures::future;

use crate::layer::Layer;

/// The output of one analysis: the reconciled entities.
///
/// Under the `usage` feature, the per-component `Usage` the run recorded
/// (one entry per recognizer and enricher, in run order).
#[derive(Debug, Clone)]
pub struct Analysis<M: Modality> {
    /// The reconciled entities, in the caller's coordinate system.
    pub entities: Vec<Entity<M>>,
    /// The enrichment artifact the analysis produced (or was seeded with): the
    /// OCR [`Layout`] / STT [`Transcription`] the recognizers read. Carried out
    /// so it can be persisted and restored for a re-run without re-enriching.
    ///
    /// [`Some`] iff an enricher ran (or a saved artifact was restored) —
    /// `Some(empty)` (an image OCR'd to no text, a silent clip) is a real
    /// enrichment, distinct from [`None`] (a modality with no enrichment, or an
    /// un-enriched payload), so it is persisted and a re-run does not re-enrich.
    ///
    /// [`Layout`]: elide_core::modality::image::Layout
    /// [`Transcription`]: elide_core::modality::audio::Transcription
    pub artifact: Option<M::Artifact>,
    /// Per-recognizer / per-enricher resource usage for this analysis.
    #[cfg(feature = "usage")]
    pub usage: Vec<Usage>,
}

impl<M: Modality> Analysis<M> {
    /// An analysis carrying `entities` and no artifact (and, under the `usage`
    /// feature, no usage yet — attach it with `with_usage`).
    pub fn new(entities: Vec<Entity<M>>) -> Self {
        Self {
            entities,
            artifact: None,
            #[cfg(feature = "usage")]
            usage: Vec::new(),
        }
    }

    /// Attach the enrichment [`artifact`](Self::artifact) the analysis produced
    /// — `Some` when it enriched (even to an empty artifact), `None` otherwise.
    #[must_use]
    pub fn with_artifact(mut self, artifact: Option<M::Artifact>) -> Self {
        self.artifact = artifact;
        self
    }

    /// Attach the per-component [`Usage`] this analysis recorded.
    #[cfg(feature = "usage")]
    #[must_use]
    pub fn with_usage(mut self, usage: Vec<Usage>) -> Self {
        self.usage = usage;
        self
    }
}

impl<M: Modality> Default for Analysis<M> {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

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
    ) -> Result<Analysis<M>> {
        // An empty catalog requests no entity types: detect nothing. Gate here,
        // the single choke point every `analyze`/`analyze_stream` entry funnels
        // through, so no enricher or recognizer runs on a scope that asked for
        // nothing — and no detected entity can then slip through unredacted.
        if ctx.catalog().is_empty() {
            // Detect nothing, but carry the seeded artifact through: a re-run
            // seeded with a prior OCR/transcript must report it back unchanged
            // so `drive` persists it, or the next re-run would re-enrich.
            return Ok(Analysis::new(Vec::new()).with_artifact(ctx.artifact().cloned()));
        }
        // Usage accumulates in run order: enrichers (sequential) first, then
        // recognizers.
        #[cfg(feature = "usage")]
        let mut usage = Vec::with_capacity(self.enrichers.len() + self.recognizers.len());
        for enricher in &self.enrichers {
            #[cfg(feature = "usage")]
            let start = Instant::now();
            let enrichment = enricher.enrich(&data, ctx).await?;
            // An enricher yields context, not counted entities, so its usage
            // carries a duration but no count.
            #[cfg(feature = "usage")]
            {
                let mut record = Usage::timed(enricher.id(), start.elapsed());
                if let Some(model) = enrichment.model_usage {
                    record = record.with_model(model);
                }
                usage.push(record);
            }
            #[cfg(not(feature = "usage"))]
            let _ = enrichment;
        }
        #[cfg(feature = "usage")]
        let (mut entities, recognizer_usage) = self.recognize(&data, ctx).await?;
        #[cfg(not(feature = "usage"))]
        let mut entities = self.recognize(&data, ctx).await?;
        #[cfg(feature = "usage")]
        usage.extend(recognizer_usage);
        ctx.stamp_languages(&mut entities);
        let reduced = self.reduce(entities);
        // Restrict the *output* to the requested catalog only after
        // reconciliation, so a strong out-of-catalog detection can subsume a
        // weak in-catalog one nested inside it before being culled itself.
        let in_catalog = ctx.catalog().retain_declared(reduced);
        let entities = Self::apply_exclusions(in_catalog, ctx.exclusions());
        // Carry the enrichment artifact out with the entities so it can be
        // persisted and restored for a re-run without re-enriching.
        let analysis = Analysis::new(entities).with_artifact(ctx.artifact().cloned());
        #[cfg(feature = "usage")]
        let analysis = analysis.with_usage(usage);
        Ok(analysis)
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
    /// the caller's modality-free assertions (languages, jurisdictions,
    /// labels, catalog). Use [`analyze_with`] to also pass per-modality
    /// region [`Annotations`] (inclusions / exclusions), or
    /// [`analyze_stream`] for an I/O-backed source that yields many chunks.
    ///
    /// [`analyze_with`]: Self::analyze_with
    /// [`analyze_stream`]: Self::analyze_stream
    /// [`Annotations`]: elide_core::recognition::annotation::Annotations
    pub async fn analyze(&self, data: M::Data, scope: &Scope) -> Result<Analysis<M>> {
        self.analyze_with(data, scope, &Annotations::new()).await
    }

    /// Analyze a single in-memory payload with both the `scope` and the
    /// caller's per-request region [`Annotations`] (inclusions / exclusions).
    ///
    /// The region-aware counterpart to [`analyze`]: `annotations` is a
    /// per-call input, not analyzer config, so it is passed here rather than
    /// stored.
    ///
    /// [`analyze`]: Self::analyze
    /// [`Annotations`]: elide_core::recognition::annotation::Annotations
    pub async fn analyze_with(
        &self,
        data: M::Data,
        scope: &Scope,
        annotations: &Annotations<M>,
    ) -> Result<Analysis<M>> {
        let mut ctx = RecognizerContext::new(scope).with_annotations(annotations);
        self.analyze_core(data, &mut ctx).await
    }

    /// Analyze one payload against a caller-supplied context, so a re-run can
    /// pre-seed the enrichment [`artifact`](RecognizerContext::artifact) and
    /// re-recognize without re-enriching — the enrichers self-skip on a present
    /// artifact. The single-payload counterpart to [`analyze_stream_in`].
    ///
    /// [`analyze_stream_in`]: Self::analyze_stream_in
    pub async fn analyze_in(
        &self,
        data: M::Data,
        ctx: &mut RecognizerContext<'_, M>,
    ) -> Result<Analysis<M>> {
        self.analyze_core(data, ctx).await
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
    /// way [`analyze`] reduces a single payload. Use [`analyze_stream_with`]
    /// to also pass per-request region [`Annotations`].
    ///
    /// Returns the first enricher, recognizer, or read error.
    ///
    /// [`Chunk`]: elide_core::modality::Chunk
    /// [`analyze`]: Self::analyze
    /// [`analyze_stream_with`]: Self::analyze_stream_with
    /// [`Annotations`]: elide_core::recognition::annotation::Annotations
    /// [`lift`]: elide_core::modality::StreamDataReader::lift
    pub async fn analyze_stream<S>(&self, source: &mut S, scope: &Scope) -> Result<Analysis<M>>
    where
        S: StreamDataReader<M>,
    {
        self.analyze_stream_with(source, scope, &Annotations::new())
            .await
    }

    /// Analyze a streamed source with both the `scope` and the caller's
    /// per-request region [`Annotations`]. The region-aware counterpart to
    /// [`analyze_stream`].
    ///
    /// [`analyze_stream`]: Self::analyze_stream
    /// [`Annotations`]: elide_core::recognition::annotation::Annotations
    pub async fn analyze_stream_with<S>(
        &self,
        source: &mut S,
        scope: &Scope,
        annotations: &Annotations<M>,
    ) -> Result<Analysis<M>>
    where
        S: StreamDataReader<M>,
    {
        self.analyze_stream_seeded(source, scope, annotations, None)
            .await
    }

    /// [`analyze_stream_with`] seeded with a prior enrichment `artifact`, so a
    /// re-run re-recognizes without re-enriching: each chunk's context is
    /// pre-seeded with `artifact`, and the enrichers self-skip because one is
    /// already present. `Some` (even an empty artifact) restores; `None` is a
    /// first pass that enriches from scratch. For the single-chunk media that
    /// produce an artifact (image, audio) this seeds the one chunk.
    ///
    /// [`analyze_stream_with`]: Self::analyze_stream_with
    pub async fn analyze_stream_in<S>(
        &self,
        source: &mut S,
        scope: &Scope,
        annotations: &Annotations<M>,
        artifact: Option<M::Artifact>,
    ) -> Result<Analysis<M>>
    where
        S: StreamDataReader<M>,
    {
        self.analyze_stream_seeded(source, scope, annotations, artifact)
            .await
    }

    /// The shared streaming core: drive `source` chunk by chunk, optionally
    /// seeding each chunk's context with `seed`, and aggregate the lifted
    /// entities plus the produced enrichment artifact.
    async fn analyze_stream_seeded<S>(
        &self,
        source: &mut S,
        scope: &Scope,
        annotations: &Annotations<M>,
        seed: Option<M::Artifact>,
    ) -> Result<Analysis<M>>
    where
        S: StreamDataReader<M>,
    {
        let mut out = Vec::new();
        // The stream's artifact: the seed when re-running (every chunk self-skips
        // and hands it back unchanged), else the one a producing chunk yields, or
        // `None` when nothing enriched. Seeded from `seed` so a re-run whose
        // chunks produce nothing new still carries the prior enrichment forward.
        let mut artifact = seed.clone();
        #[cfg(feature = "usage")]
        let mut usage = Vec::new();
        while let Some(chunk) = source.read_next().await? {
            let mut ctx = RecognizerContext::new(scope)
                .with_annotations(annotations)
                .with_context_hints(chunk.hints.clone());
            if let Some(seed) = &seed {
                ctx = ctx.with_artifact(seed.clone());
            }
            let analysis = self.analyze_core(chunk.data.clone(), &mut ctx).await?;
            // Usage accrues across chunks: each chunk re-runs every recognizer
            // and enricher, so the stream's total is the sum of its chunks'.
            #[cfg(feature = "usage")]
            usage.extend(analysis.usage);
            // A chunk that produced a *new* artifact (`Some`, and not just the
            // seed handed straight back) owns the stream's artifact. The media
            // that produce one (image, audio) are single-chunk, so exactly one
            // chunk does this; a plain text/tabular stream produces none and
            // keeps the seed. A multi-chunk *tokenizing* stream would produce a
            // per-chunk artifact on more than one chunk — unsupported, because
            // one stream-level artifact cannot represent per-chunk tokens; the
            // assert catches that the day such an enricher is added.
            if analysis.artifact.is_some() && analysis.artifact != seed {
                debug_assert!(
                    artifact == seed,
                    "a multi-chunk stream produced more than one enrichment artifact; \
                     per-chunk artifacts are not representable at the stream level",
                );
                artifact = analysis.artifact;
            }
            out.extend(
                analysis
                    .entities
                    .into_iter()
                    .filter_map(|entity| source.lift(&chunk, entity)),
            );
        }
        let analysis = Analysis::new(out).with_artifact(artifact);
        #[cfg(feature = "usage")]
        let analysis = analysis.with_usage(usage);
        Ok(analysis)
    }

    /// Run every recognizer over `data` concurrently and collect their
    /// entities. Under the `usage` feature it also returns a [`Usage`] per
    /// recognizer (its id, wall-clock time, entity count, and any model/token
    /// detail it returned). The first error is returned (fail-fast).
    ///
    /// Recognizers borrow `data` and `ctx`, so they are joined in place
    /// rather than spawned onto the runtime.
    #[cfg(feature = "usage")]
    async fn recognize(
        &self,
        data: &M::Data,
        ctx: &RecognizerContext<'_, M>,
    ) -> Result<(Vec<Entity<M>>, Vec<Usage>)> {
        if self.recognizers.is_empty() {
            return Ok((Vec::new(), Vec::new()));
        }

        // Time each recognizer inside its own future so the measure reflects
        // that recognizer alone; `join_all` polls them on one task (no spawn),
        // so the timings are concurrent-but-in-place, matching how they run.
        // Each future returns this recognizer's id, its elapsed millis, and
        // its `Recognition`; `join_all` yields one per recognizer in order.
        let futures = self.recognizers.iter().map(|recognizer| {
            let id = recognizer.id();
            async move {
                let start = Instant::now();
                let recognition: Recognition<M> = recognizer.recognize(data, ctx).await?;
                let elapsed = start.elapsed();
                Result::<_>::Ok((id, elapsed, recognition))
            }
        });

        let mut entities = Vec::new();
        let mut usage = Vec::with_capacity(self.recognizers.len());
        for found in future::join_all(futures).await {
            let (id, elapsed, recognition): (RecognizerId, Duration, Recognition<M>) = found?;
            let count = recognition.entities.len() as u64;
            let mut record = Usage::new(id, elapsed, count);
            if let Some(model) = recognition.model_usage {
                record = record.with_model(model);
            }
            usage.push(record);
            entities.extend(recognition.entities);
        }
        Ok((entities, usage))
    }

    /// Run every recognizer over `data` concurrently and collect their
    /// entities. The first error is returned (fail-fast).
    ///
    /// Recognizers borrow `data` and `ctx`, so they are joined in place
    /// rather than spawned onto the runtime.
    #[cfg(not(feature = "usage"))]
    async fn recognize(
        &self,
        data: &M::Data,
        ctx: &RecognizerContext<'_, M>,
    ) -> Result<Vec<Entity<M>>> {
        let futures = self
            .recognizers
            .iter()
            .map(|recognizer| recognizer.recognize(data, ctx));

        let mut entities = Vec::new();
        for found in future::join_all(futures).await {
            let recognition: Recognition<M> = found?;
            entities.extend(recognition.entities);
        }
        Ok(entities)
    }
}

impl<M: Modality> Default for Analyzer<M> {
    fn default() -> Self {
        Self::new()
    }
}
