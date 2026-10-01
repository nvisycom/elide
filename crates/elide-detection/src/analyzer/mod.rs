//! The [`Analyzer`]: the "find" engine.
//!
//! Wraps enrichers, recognizers, and a deduplication pipeline into one
//! Presidio-style entry point. Enrichers, recognizers, and [`Layer`]s are added
//! with the `with_*` builders; the `analyze*` entries run three phases in order:
//! enrich (sequential), recognize (concurrent), reduce (the layers), returning a
//! clean entity set.
//!
//! Every entry — a single in-memory payload, one streamed source, or many
//! streamed sources — funnels through the one engine, [`analyze_sources`]: it
//! drains each source's chunks into subjects, enriches the whole flat batch in
//! one pass (so an OCR/STT enricher issues one `call_batch` across every
//! subject), recognizes each, and re-aggregates per source. A text source
//! contributes many subjects and a single-chunk image/audio source one — the
//! same path, only the count differs. The [`Analysis`] result type lives in
//! [`analysis`].
//!
//! [`Layer`]: crate::layer::Layer
//! [`analyze_sources`]: Analyzer::analyze_sources

mod analysis;

use std::sync::Arc;

use elide_core::enrichment::Enricher;
use elide_core::entity::Entity;
use elide_core::modality::{Chunk, Modality, ModalityLocation, StreamDataReader};
use elide_core::recognition::annotation::{Annotations, Exclusion};
use elide_core::recognition::{Context, Recognizer, Scope, Subject};
use elide_core::{Error, ErrorKind, Result};
use futures::future;

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

    // --- Public entry points: thin adapters onto the engine below. ---

    /// Analyze a single in-memory payload in the given scope.
    ///
    /// Runs the full analysis pipeline over `data`, with `scope` supplying the
    /// caller's modality-free assertions (languages, jurisdictions, labels,
    /// catalog). Use [`analyze_with`](Self::analyze_with) to also pass
    /// per-modality region [`Annotations`], or [`analyze_stream`](Self::analyze_stream)
    /// for an I/O-backed source that yields many chunks.
    pub async fn analyze(&self, data: M::Data, scope: &Scope) -> Result<Analysis<M>> {
        self.analyze_with(data, scope, &Annotations::new()).await
    }

    /// Analyze a single in-memory payload with both the `scope` and the caller's
    /// per-request region [`Annotations`].
    ///
    /// The region-aware counterpart to [`analyze`](Self::analyze): `annotations`
    /// is a per-call input, not analyzer config, so it is passed here.
    pub async fn analyze_with(
        &self,
        data: M::Data,
        scope: &Scope,
        annotations: &Annotations<M>,
    ) -> Result<Analysis<M>> {
        self.analyze_in(data, scope, annotations, None).await
    }

    /// Analyze one payload, optionally pre-seeded with a prior enrichment `seed`
    /// so a re-run re-recognizes without re-enriching (the enrichers self-skip on
    /// a present artifact). The single-payload counterpart to
    /// [`analyze_stream_in`](Self::analyze_stream_in).
    ///
    /// An in-memory payload is one subject with no source coordinates to lift back
    /// to, so it runs through the engine's per-subject core directly.
    pub async fn analyze_in(
        &self,
        data: M::Data,
        scope: &Scope,
        annotations: &Annotations<M>,
        seed: Option<M::Artifact>,
    ) -> Result<Analysis<M>> {
        let ctx = Context::new(scope).with_annotations(annotations);
        if ctx.catalog().is_empty() {
            return Ok(Analysis::seeded(seed));
        }
        let mut subject = Subject::new(data);
        if let Some(seed) = seed {
            subject = subject.with_artifact(seed);
        }
        self.analyze_subject(&mut subject, &ctx).await
    }

    /// Analyze a streamed source end to end, returning entities in the source's
    /// own coordinate system.
    ///
    /// The [`analyze`](Self::analyze) counterpart for I/O-backed sources (a
    /// decoded codec document, say): the caller never sees a chunk or a
    /// recognizer-local coordinate. Use [`analyze_stream_with`](Self::analyze_stream_with)
    /// to also pass per-request region [`Annotations`].
    pub async fn analyze_stream<S>(&self, source: &mut S, scope: &Scope) -> Result<Analysis<M>>
    where
        S: StreamDataReader<M> + ?Sized,
    {
        self.analyze_stream_with(source, scope, &Annotations::new())
            .await
    }

    /// Analyze a streamed source with both the `scope` and the caller's
    /// per-request region [`Annotations`]. The region-aware counterpart to
    /// [`analyze_stream`](Self::analyze_stream).
    pub async fn analyze_stream_with<S>(
        &self,
        source: &mut S,
        scope: &Scope,
        annotations: &Annotations<M>,
    ) -> Result<Analysis<M>>
    where
        S: StreamDataReader<M> + ?Sized,
    {
        self.analyze_stream_in(source, scope, annotations, None)
            .await
    }

    /// [`analyze_stream_with`](Self::analyze_stream_with) seeded with a prior
    /// enrichment `seed`, so a re-run re-recognizes without re-enriching: the
    /// source's subjects are pre-seeded, and the enrichers self-skip because one
    /// is already present. `Some` (even an empty artifact) restores; `None` is a
    /// first pass that enriches from scratch.
    pub async fn analyze_stream_in<S>(
        &self,
        source: &mut S,
        scope: &Scope,
        annotations: &Annotations<M>,
        seed: Option<M::Artifact>,
    ) -> Result<Analysis<M>>
    where
        S: StreamDataReader<M> + ?Sized,
    {
        let mut sources = [source];
        let mut analyses = self
            .analyze_sources(&mut sources, scope, annotations, vec![seed])
            .await?;
        // One source in, one analysis out.
        Ok(analyses
            .pop()
            .expect("analyze_sources returns one analysis per source"))
    }

    /// Analyze several streamed sources together, coalescing enrichment into one
    /// batched provider round-trip, and return one [`Analysis`] per source in
    /// order. Each source is paired with its prior enrichment `seeds` entry.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) when
    /// `sources` and `seeds` differ in length; otherwise the first enrichment or
    /// recognition error.
    pub async fn analyze_streams<S>(
        &self,
        sources: &mut [&mut S],
        scope: &Scope,
        annotations: &Annotations<M>,
        seeds: Vec<Option<M::Artifact>>,
    ) -> Result<Vec<Analysis<M>>>
    where
        S: StreamDataReader<M> + ?Sized,
    {
        self.analyze_sources(sources, scope, annotations, seeds)
            .await
    }

    // --- The engine: the one core every entry funnels through. ---

    /// The one analysis engine: analyze every `source` together and return one
    /// [`Analysis`] per source, in order.
    ///
    /// Each source is seeded from its paired `seeds` entry (a re-run's restored
    /// artifact, or `None` for a first pass). Every source's chunks become
    /// subjects in one flat batch, so enrichment coalesces across all of them; a
    /// text source contributes many subjects and a single-chunk image/audio source
    /// one — the same path, only the count differs. Each source's entities are
    /// lifted back through its own chunks and aggregated, and its single enrichment
    /// artifact (image/audio produce one; text none) is carried out.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) when
    /// `sources` and `seeds` differ in length (the caller must supply one seed
    /// per source); otherwise the first enrichment or recognition error.
    async fn analyze_sources<S>(
        &self,
        sources: &mut [&mut S],
        scope: &Scope,
        annotations: &Annotations<M>,
        seeds: Vec<Option<M::Artifact>>,
    ) -> Result<Vec<Analysis<M>>>
    where
        S: StreamDataReader<M> + ?Sized,
    {
        if sources.len() != seeds.len() {
            return Err(Error::new(
                ErrorKind::MalformedInput,
                format!(
                    "analyze_streams: {} sources but {} seeds; expected one seed per source",
                    sources.len(),
                    seeds.len()
                ),
            ));
        }
        let ctx = Context::new(scope).with_annotations(annotations);

        // An empty catalog requests no entity types: detect nothing, but carry
        // each source's seed through. Gate before any read/enrich/recognize.
        if ctx.catalog().is_empty() {
            return Ok(seeds.into_iter().map(Analysis::seeded).collect());
        }

        // Drain every source's chunks into one flat batch, kept as parallel
        // vectors so `enrich_batch` gets a plain `&mut [Subject]` slice with no
        // clone. `origins[i]` records the source and chunk `subjects[i]` came from,
        // so its entities lift back to that source's coordinates. A source with no
        // chunk contributes nothing and keeps its seed (handled at aggregation).
        let mut subjects: Vec<Subject<M>> = Vec::new();
        let mut origins: Vec<(usize, Chunk<M>)> = Vec::new();
        for (index, (source, seed)) in sources.iter().zip(&seeds).enumerate() {
            for chunk in source.chunks()? {
                let mut subject = Subject::new(chunk.data.clone()).with_hints(chunk.hints.clone());
                if let Some(seed) = seed {
                    subject = subject.with_artifact(seed.clone());
                }
                subjects.push(subject);
                origins.push((index, chunk));
            }
        }

        // Batched enrichment: each enricher sees every subject, so an OCR/STT
        // enricher coalesces its provider round-trips into one `call_batch`.
        for enricher in &self.enrichers {
            enricher.enrich_batch(&mut subjects, &ctx).await?;
        }

        // Recognize the whole subject batch at once — every recognizer runs
        // concurrently, each over all subjects, so a provider-backed recognizer
        // (NER) coalesces its round-trips into one `call_batch`. `raw[i]` is
        // subject `i`'s entities from every recognizer combined.
        let raw = self.recognize_batch(&subjects, &ctx).await?;

        // Reduce each subject and aggregate per source: lift its entities back
        // through its own chunk, and carry the source's one enrichment artifact.
        // Each source's analysis starts from its seed; `seeds` is retained
        // read-only for the single-artifact invariant.
        let mut analyses: Vec<Analysis<M>> = seeds.iter().cloned().map(Analysis::seeded).collect();
        for ((subject, (source, chunk)), entities) in subjects.into_iter().zip(origins).zip(raw) {
            let analysis = self.finish_subject(&subject, &ctx, entities);
            let lifted = analysis
                .entities
                .into_iter()
                .filter_map(|entity| sources[source].lift(&chunk, entity));
            analyses[source].entities.extend(lifted);
            // A chunk that produced a *new* artifact (`Some`, and not the seed
            // handed straight back) owns its source's artifact. The media that
            // produce one (image, audio) are single-chunk, so exactly one chunk
            // does this per source; text produces none and keeps the seed. A
            // multi-chunk stream producing more than one is unrepresentable (one
            // stream-level artifact cannot hold per-chunk state); the assert
            // catches that the day such an enricher is added.
            if analysis.artifact.is_some() && analysis.artifact != seeds[source] {
                debug_assert!(
                    analyses[source].artifact == seeds[source],
                    "a multi-chunk stream produced more than one enrichment artifact; \
                     per-chunk artifacts are not representable at the stream level",
                );
                analyses[source].artifact = analysis.artifact;
            }
        }
        Ok(analyses)
    }

    /// Analyze one in-memory subject: enrich it, then recognize. The degenerate
    /// batch of one with no source coordinates to lift back to (used by the
    /// in-memory [`analyze`](Self::analyze) entries).
    async fn analyze_subject(
        &self,
        subject: &mut Subject<M>,
        ctx: &Context<'_, M>,
    ) -> Result<Analysis<M>> {
        for enricher in &self.enrichers {
            enricher.enrich(subject, ctx).await?;
        }
        let mut raw = self
            .recognize_batch(std::slice::from_ref(subject), ctx)
            .await?;
        let entities = raw.pop().unwrap_or_default();
        Ok(self.finish_subject(subject, ctx, entities))
    }

    /// Recognize the whole `subjects` batch: every recognizer runs concurrently,
    /// each over all subjects, and the per-subject entities from every recognizer
    /// are combined. Returns one entity list per subject, in order.
    ///
    /// Concurrency is over recognizers (joined in place, not spawned, since they
    /// borrow the subjects and `ctx`); a provider-backed recognizer coalesces its
    /// per-subject calls into one [`Recognizer::recognize_batch`]. The first error
    /// is returned (fail-fast).
    async fn recognize_batch(
        &self,
        subjects: &[Subject<M>],
        ctx: &Context<'_, M>,
    ) -> Result<Vec<Vec<Entity<M>>>> {
        let futures = self
            .recognizers
            .iter()
            .map(|recognizer| recognizer.recognize_batch(subjects, ctx));
        // Each recognizer returns one entity list per subject; transpose so
        // `entities[s]` gathers every recognizer's entities for subject s.
        let mut entities: Vec<Vec<Entity<M>>> = subjects.iter().map(|_| Vec::new()).collect();
        for found in future::join_all(futures).await {
            for (slot, found_for_subject) in entities.iter_mut().zip(found?) {
                slot.extend(found_for_subject);
            }
        }
        Ok(entities)
    }

    /// Turn an already-enriched, already-recognized `subject`'s raw `entities`
    /// into its [`Analysis`]: stamp languages, reduce, restrict to the catalog,
    /// apply exclusions, and carry the enrichment artifact out.
    fn finish_subject(
        &self,
        subject: &Subject<M>,
        ctx: &Context<'_, M>,
        mut entities: Vec<Entity<M>>,
    ) -> Analysis<M> {
        ctx.languages(subject).stamp(&mut entities);
        let reduced = self.reduce(entities);
        // Restrict the *output* to the requested catalog only after reconciliation,
        // so a strong out-of-catalog detection can subsume a weak in-catalog one
        // nested inside it before being culled itself.
        let in_catalog = ctx.catalog().retain_declared(reduced);
        let entities = Self::apply_exclusions(in_catalog, ctx.exclusions());
        // Carry the enrichment artifact out with the entities so it can be
        // persisted and restored for a re-run without re-enriching.
        Analysis::new(entities).with_artifact(subject.artifact().cloned())
    }

    /// Run every deduplication layer in order over `entities`, threading each
    /// layer's kept output into the next and returning the survivors.
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
    /// Runs after deduplication so it culls the reconciled set, not per-recognizer
    /// duplicates. A no-op when no exclusions are asserted.
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
}

impl<M: Modality> Default for Analyzer<M> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use elide_core::entity::audit::{AuditEvent, AuditLog, PatternEvent};
    use elide_core::entity::{Entity, Label, LabelCatalog, LabelRef};
    use elide_core::mocks::MockRecognizer;
    use elide_core::modality::Chunk;
    use elide_core::modality::text::{Text, TextData, TextLocation};
    use elide_core::primitive::{ComponentId, Confidence};

    use super::*;

    /// A [`Text`] source over a fixed list of chunks, with the identity lift (a
    /// chunk's decoded range already addresses the source), for driving the engine
    /// without a codec.
    struct VecTextSource(Vec<Chunk<Text>>);

    impl VecTextSource {
        /// A single-chunk source whose one chunk is `text` at `[0, len)`.
        fn single(text: &str) -> Self {
            let len = text.len();
            let chunk = Chunk::new(TextLocation::new(0, len), TextData::new(text.to_owned()));
            Self(vec![chunk])
        }
    }

    impl StreamDataReader<Text> for VecTextSource {
        fn chunks(&self) -> Result<Vec<Chunk<Text>>> {
            Ok(self.0.clone())
        }

        fn lift(&self, chunk: &Chunk<Text>, mut entity: Entity<Text>) -> Option<Entity<Text>> {
            // A recognizer's finding is chunk-local (offset into the chunk's
            // text); rebase it onto the chunk's start, as a real text stream does.
            let start = chunk.location.range().map(|r| r.start).unwrap_or(0);
            let local = entity.location.range()?.clone();
            entity.location = TextLocation::new(start + local.start, start + local.end);
            Some(entity)
        }
    }

    /// An enricher that records how many times it was batched and how many
    /// subjects it saw in total, to prove the engine coalesces across sources.
    /// Counters are `Arc`-shared so the test can read them after the enricher is
    /// moved into the analyzer.
    #[derive(Clone, Default)]
    struct SpyEnricher {
        batches: Arc<AtomicUsize>,
        subjects: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl Enricher<Text> for SpyEnricher {
        fn id(&self) -> ComponentId {
            ComponentId::new("spy", "1")
        }

        async fn enrich(
            &self,
            _subject: &mut Subject<Text>,
            _ctx: &Context<'_, Text>,
        ) -> Result<()> {
            Ok(())
        }

        async fn enrich_batch(
            &self,
            subjects: &mut [Subject<Text>],
            _ctx: &Context<'_, Text>,
        ) -> Result<()> {
            self.batches.fetch_add(1, Ordering::Relaxed);
            self.subjects.fetch_add(subjects.len(), Ordering::Relaxed);
            Ok(())
        }
    }

    fn detected(label: &str, loc: (usize, usize)) -> Entity<Text> {
        let label = LabelRef::new(label.to_owned());
        let location = TextLocation::new(loc.0, loc.1);
        let confidence = Confidence::new(0.9).unwrap();
        let event = AuditEvent::pattern(
            "mock",
            confidence,
            location.clone(),
            PatternEvent {
                name: label.as_str().into(),
                ..PatternEvent::default()
            },
        );
        Entity::new(label, location, AuditLog::new(event))
    }

    fn scope_for(ids: &[&str]) -> Scope {
        let catalog: LabelCatalog = ids.iter().map(|id| Label::new(*id, *id)).collect();
        Scope::new().with_catalog(catalog)
    }

    #[tokio::test]
    async fn analyze_streams_coalesces_enrichment_across_sources() {
        let spy = SpyEnricher::default();
        let counters = spy.clone();
        let analyzer = Analyzer::<Text>::new()
            .with_enricher(spy)
            .with_recognizer(MockRecognizer::new(vec![detected("EMAIL", (0, 3))]));

        // Two single-chunk sources and one two-chunk source: four subjects total.
        let mut a = VecTextSource::single("aaa");
        let mut b = VecTextSource::single("bbb");
        let mut c = VecTextSource(vec![
            Chunk::new(TextLocation::new(0, 3), TextData::new("ccc".to_owned())),
            Chunk::new(TextLocation::new(3, 6), TextData::new("ddd".to_owned())),
        ]);
        let mut sources: Vec<&mut VecTextSource> = vec![&mut a, &mut b, &mut c];

        let analyses = analyzer
            .analyze_streams(
                &mut sources,
                &scope_for(&["EMAIL"]),
                &Default::default(),
                vec![None, None, None],
            )
            .await
            .unwrap();

        // One batched enrichment covered all four subjects across the three
        // sources — not one enrich per source.
        assert_eq!(counters.batches.load(Ordering::Relaxed), 1);
        assert_eq!(counters.subjects.load(Ordering::Relaxed), 4);
        // One analysis per source, in order.
        assert_eq!(analyses.len(), 3);
    }

    #[tokio::test]
    async fn analyze_streams_aggregates_entities_per_source() {
        // A recognizer that flags bytes 0..3 as EMAIL on every chunk. The two-chunk
        // source lifts the second chunk's finding to its source offset (3..6).
        let analyzer = Analyzer::<Text>::new()
            .with_recognizer(MockRecognizer::new(vec![detected("EMAIL", (0, 3))]));

        let mut a = VecTextSource::single("aaa");
        let mut c = VecTextSource(vec![
            Chunk::new(TextLocation::new(0, 3), TextData::new("ccc".to_owned())),
            Chunk::new(TextLocation::new(3, 6), TextData::new("ddd".to_owned())),
        ]);
        let mut sources: Vec<&mut VecTextSource> = vec![&mut a, &mut c];

        let analyses = analyzer
            .analyze_streams(
                &mut sources,
                &scope_for(&["EMAIL"]),
                &Default::default(),
                vec![None, None],
            )
            .await
            .unwrap();

        // Source A (one chunk) has one finding; source C (two chunks) has two, the
        // second lifted to the second chunk's source offset.
        assert_eq!(analyses[0].entities.len(), 1);
        assert_eq!(analyses[1].entities.len(), 2);
        let ranges: Vec<_> = analyses[1]
            .entities
            .iter()
            .filter_map(|e| e.location.range().cloned())
            .collect();
        assert!(ranges.contains(&(0..3)));
        assert!(ranges.contains(&(3..6)));
    }

    /// A seeds vector that does not match the sources one-for-one is malformed
    /// caller input: it must error rather than silently pair the wrong seeds or
    /// leave the extra sources unread (and their PII unreported).
    #[tokio::test]
    async fn analyze_streams_rejects_a_seed_count_mismatch() {
        let analyzer = Analyzer::<Text>::new()
            .with_recognizer(MockRecognizer::new(vec![detected("EMAIL", (0, 3))]));

        let mut a = VecTextSource::single("aaa");
        let mut b = VecTextSource::single("bbb");
        let mut sources: Vec<&mut VecTextSource> = vec![&mut a, &mut b];

        // Two sources, one seed.
        let err = analyzer
            .analyze_streams(
                &mut sources,
                &scope_for(&["EMAIL"]),
                &Default::default(),
                vec![None],
            )
            .await
            .expect_err("mismatched seed count is malformed input");
        assert_eq!(err.kind(), ErrorKind::MalformedInput);
    }
}
