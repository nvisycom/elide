//! The [`Analyzer`]: the "find" engine.
//!
//! Wraps enrichers, recognizers, and a layer pipeline into one
//! Presidio-style entry point. Enrichers, recognizers, and [`Layer`]s are added
//! with the `with_*` builders; the `analyze*` entries run three phases in order:
//! enrich (sequential), recognize (concurrent), reduce (the layers, then the
//! catalog), returning the findings, with everything the reduction dropped
//! kept beside them, suppressed, so their trails say why.
//!
//! Streamed sources, one or many, run through one engine, [`analyze_streams`]:
//! it drains each source's chunks into subjects, enriches the whole flat batch
//! in one pass (so an OCR/STT enricher issues one `call_batch` across every
//! subject), recognizes each, and re-aggregates per source. A text source
//! contributes many subjects and a single-chunk image/audio source one — the
//! same path, only the count differs. [`analyze_stream`] is its one-source form,
//! and [`analyze`] runs a single in-memory payload, which has no chunks to lift
//! back, straight through the per-subject core. The [`Analysis`] result type
//! lives in [`analysis`].
//!
//! [`Layer`]: crate::layer::Layer
//! [`analyze_streams`]: Analyzer::analyze_streams
//! [`analyze_stream`]: Analyzer::analyze_stream
//! [`analyze`]: Analyzer::analyze

mod analysis;

use std::iter;
use std::sync::Arc;

use elide_core::enrichment::Enricher;
use elide_core::entity::Entity;
use elide_core::entity::audit::{AuditEvent, Dropped};
use elide_core::modality::{Chunk, ChunkReader, Modality};
use elide_core::recognition::{Context, Recognizer, Scope, Subject};
use elide_core::{Component, Error, Result};
use futures::future;

pub use self::analysis::Analysis;
use crate::batch::expect_one_per_input;
use crate::layer::Layer;
use crate::layer::filter::CatalogFilter;

/// The find engine: enrichers, recognizers, and layers, in one call.
///
/// Generic over the [`Modality`] `M`. Enrichers, recognizers, and
/// layers are added with [`with_enricher`], [`with_recognizer`], and
/// [`with_layer`], each in the order it should run. [`analyze`] runs the three
/// phases and returns an [`Analysis`] carrying the reconciled entities, and
/// every entity a layer or the request's catalog dropped, suppressed by a
/// [`Dropped`] event on its trail (or discarded, see [`discard_dropped`]).
///
/// [`discard_dropped`]: Analyzer::discard_dropped
///
/// ```ignore
/// let analysis = Analyzer::new()
///     .with_enricher(lingua)
///     .with_recognizer(us_phone)
///     .with_recognizer(ner)
///     .with_layer(FuseLayer::max())
///     .with_layer(ReconcileLayer::structural())
///     .with_layer(ThresholdLayer::default())
///     .analyze(data, &Scope::new().with_catalog(LabelCatalog::with_builtins()))
///     .await?;
/// let found = analysis.entities.iter().filter(|entity| !entity.is_dropped());
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
    retain_dropped: bool,
}

impl<M: Modality> Analyzer<M> {
    /// An analyzer with no enrichers, recognizers, or layers.
    pub fn new() -> Self {
        Self {
            enrichers: Vec::new(),
            recognizers: Vec::new(),
            layers: Vec::new(),
            retain_dropped: true,
        }
    }

    /// Add an enricher; enrichers run in order, before any recognizer.
    ///
    /// They run sequentially, so a recognizer sees what they wrote onto the
    /// input.
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

    /// Append a layer. Layers run in the order added, after
    /// detection.
    #[must_use]
    pub fn with_layer<L: Layer<M> + 'static>(mut self, layer: L) -> Self {
        self.layers.push(Arc::new(layer));
        self
    }

    /// Discard what the reduction drops instead of keeping it.
    ///
    /// By default every dropped entity stays in the [`Analysis`], its trail
    /// holding a [`Dropped`] event that names the layer and, for a lost
    /// overlap, the winner: redaction leaves it alone, and a reviewer can read
    /// why it is missing or confirm it. A low threshold over a noisy
    /// recognizer can drop many more candidates than it keeps; discarding
    /// them keeps the analysis to its findings.
    #[must_use]
    pub fn discard_dropped(mut self) -> Self {
        self.retain_dropped = false;
        self
    }

    /// Analyze a single in-memory payload in the given scope.
    ///
    /// Runs the full analysis pipeline over `data`, with `scope` supplying the
    /// caller's modality-free assertions (languages, jurisdictions, labels,
    /// catalog). Use [`analyze_stream`](Self::analyze_stream) for an I/O-backed
    /// source that yields many chunks.
    pub async fn analyze(&self, data: M::Data, scope: &Scope) -> Result<Analysis<M>> {
        let ctx = Context::new(scope);
        if ctx.catalog().is_empty() {
            return Ok(Analysis::default());
        }
        self.analyze_subject(&mut Subject::new(data), &ctx).await
    }

    /// Analyze a streamed source end to end, in the source's own coordinates.
    ///
    /// The [`analyze`](Self::analyze) counterpart for I/O-backed sources (a
    /// decoded codec document, say): the caller never sees a chunk or a
    /// recognizer-local coordinate. The one-source, first-pass form of
    /// [`analyze_streams`](Self::analyze_streams).
    pub async fn analyze_stream<S>(&self, source: &mut S, scope: &Scope) -> Result<Analysis<M>>
    where
        S: ChunkReader<M> + ?Sized,
    {
        let mut analyses = self
            .analyze_streams(&mut [source], scope, vec![None])
            .await?;
        Ok(analyses.pop().expect("one analysis per source"))
    }

    /// Analyze several streamed sources together, one [`Analysis`] per source.
    ///
    /// The analyses come back in source order.
    ///
    /// Each source is seeded from its paired `seeds` entry: a prior run's
    /// enrichment artifact, so a re-run re-recognizes without re-enriching (the
    /// enrichers skip a subject that already has one), or `None` for a first
    /// pass. Every source's chunks become
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
    pub async fn analyze_streams<S>(
        &self,
        sources: &mut [&mut S],
        scope: &Scope,
        seeds: Vec<Option<M::Artifact>>,
    ) -> Result<Vec<Analysis<M>>>
    where
        S: ChunkReader<M> + ?Sized,
    {
        if sources.len() != seeds.len() {
            return Err(Error::malformed(format!(
                "analyze_streams: {} sources but {} seeds; expected one seed per source",
                sources.len(),
                seeds.len()
            )));
        }
        let ctx = Context::new(scope);

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
        let mut batch: Vec<&mut Subject<M>> = subjects.iter_mut().collect();
        for enricher in &self.enrichers {
            enricher.enrich_batch(&mut batch, &ctx).await?;
        }

        // Recognize the whole subject batch at once — every recognizer runs
        // concurrently, each over all subjects, so a provider-backed recognizer
        // (NER) coalesces its round-trips into one `call_batch`. `raw[i]` is
        // subject `i`'s entities from every recognizer combined.
        let batch: Vec<&Subject<M>> = subjects.iter().collect();
        let raw = self.recognize_batch(&batch, &ctx).await?;

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
        ctx: &Context<'_>,
    ) -> Result<Analysis<M>> {
        for enricher in &self.enrichers {
            enricher.enrich(subject, ctx).await?;
        }
        let mut raw = self.recognize_batch(&[&*subject], ctx).await?;
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
        subjects: &[&Subject<M>],
        ctx: &Context<'_>,
    ) -> Result<Vec<Vec<Entity<M>>>> {
        let futures = self.recognizers.iter().map(|recognizer| async move {
            let found = recognizer.recognize_batch(subjects, ctx).await?;
            expect_one_per_input(recognizer.id(), found, subjects.len())
        });
        // Each recognizer returns one entity list per subject; transpose so
        // `entities[s]` gathers every recognizer's entities for subject s. The
        // recognizers are arbitrary (registered via `with_recognizer`), so this
        // aggregation boundary enforces their one-list-per-subject contract: a
        // mismatched count would otherwise let the zip silently under- or
        // over-attribute entities.
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
    /// and carry the enrichment artifact out.
    fn finish_subject(
        &self,
        subject: &Subject<M>,
        ctx: &Context<'_>,
        mut entities: Vec<Entity<M>>,
    ) -> Analysis<M> {
        ctx.languages(subject).stamp(&mut entities);
        let entities = self.reduce(entities, ctx);
        // Carry the enrichment artifact out with the entities so it can be
        // persisted and restored for a re-run without re-enriching.
        Analysis::new(entities).with_artifact(subject.artifact().cloned())
    }

    /// Run every layer in order over `entities`, then the [`CatalogFilter`],
    /// threading each one's kept entities into the next: the survivors, then
    /// everything dropped, in the order dropped, each with a [`Dropped`] event
    /// naming the layer that dropped it (unless the analyzer discards them).
    ///
    /// The layer is named here, not by the layer itself, so no layer can
    /// misname what it drops.
    ///
    /// The catalog runs last, so a strong out-of-catalog detection can subsume
    /// a weak in-catalog one nested inside it before being culled itself.
    fn reduce(&self, mut entities: Vec<Entity<M>>, ctx: &Context<'_>) -> Vec<Entity<M>> {
        let before = entities.len();
        let mut dropped = 0usize;
        let mut dropped_entities = Vec::new();
        let layers = self
            .layers
            .iter()
            .map(|layer| &**layer)
            .chain(iter::once(&CatalogFilter as &dyn Layer<M>));
        for layer in layers {
            let output = layer.apply(entities, ctx);
            dropped += output.dropped.len();
            if self.retain_dropped {
                let by = layer.id();
                for (mut entity, superseded_by) in output.dropped {
                    let reason = Dropped {
                        by: by.clone(),
                        superseded_by,
                    };
                    let confidence = entity.confidence();
                    entity.record(AuditEvent::dropped(reason, confidence));
                    dropped_entities.push(entity);
                }
            }
            entities = output.kept;
        }
        tracing::debug!(
            modality = M::NAME,
            before,
            after = entities.len(),
            dropped,
            "layers complete"
        );
        entities.extend(dropped_entities);
        entities
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

    use elide_core::entity::audit::{AuditEvent, Pattern, PatternRule};
    use elide_core::entity::{Birth, Entity, Label, LabelCatalog, LabelRef};
    use elide_core::mocks::MockRecognizer;
    use elide_core::modality::Chunk;
    use elide_core::modality::text::{Text, TextData, TextLocation};
    use elide_core::primitive::Confidence;
    use elide_core::{Component, ErrorKind};

    use super::*;
    use crate::layer::filter::ThresholdLayer;

    /// A [`Text`] source over a fixed list of chunks, with the identity lift (a
    /// chunk's decoded range already addresses the source), for driving the engine
    /// without a codec.
    struct VecTextSource(Vec<Chunk<Text>>);

    impl VecTextSource {
        /// A single-chunk source whose one chunk is `text` at `[0, len)`.
        fn single(text: &str) -> Self {
            let len = text.len();
            let chunk = Chunk::new(TextLocation::inline(0..len), TextData::new(text.to_owned()));
            Self(vec![chunk])
        }
    }

    impl ChunkReader<Text> for VecTextSource {
        fn chunks(&self) -> Result<Vec<Chunk<Text>>> {
            Ok(self.0.clone())
        }

        fn lift(&self, chunk: &Chunk<Text>, entity: Entity<Text>) -> Option<Entity<Text>> {
            // A recognizer's finding is chunk-local (offset into the chunk's
            // text); rebase it onto the chunk's start, as a real text stream does.
            let start = chunk.location.range().map(|r| r.start).unwrap_or(0);
            entity.offset(start)
        }
    }

    /// An enricher that records how many times it was batched and how many
    /// subjects it saw in total, to prove the engine coalesces across sources.
    /// Counters are `Arc`-shared so the test can read them after the enricher is
    /// moved into the analyzer.
    #[derive(Clone, Default, Component)]
    #[component(name = "spy", version = "1")]
    struct SpyEnricher {
        batches: Arc<AtomicUsize>,
        subjects: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl Enricher<Text> for SpyEnricher {
        async fn enrich(&self, _subject: &mut Subject<Text>, _ctx: &Context<'_>) -> Result<()> {
            Ok(())
        }

        async fn enrich_batch(
            &self,
            subjects: &mut [&mut Subject<Text>],
            _ctx: &Context<'_>,
        ) -> Result<()> {
            self.batches.fetch_add(1, Ordering::Relaxed);
            self.subjects.fetch_add(subjects.len(), Ordering::Relaxed);
            Ok(())
        }
    }

    fn detected(label: &str, loc: (usize, usize)) -> Entity<Text> {
        let label = LabelRef::new(label.to_owned());
        let location = TextLocation::inline(loc.0..loc.1);
        let confidence = Confidence::new(0.9).unwrap();
        Entity::new(Birth::new(
            "mock",
            confidence,
            Pattern::new(
                label.clone(),
                location,
                PatternRule {
                    name: label.as_str().into(),
                    ..PatternRule::default()
                },
            ),
        ))
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
            Chunk::new(TextLocation::inline(0..3), TextData::new("ccc".to_owned())),
            Chunk::new(TextLocation::inline(3..6), TextData::new("ddd".to_owned())),
        ]);
        let mut sources: Vec<&mut VecTextSource> = vec![&mut a, &mut b, &mut c];

        let analyses = analyzer
            .analyze_streams(&mut sources, &scope_for(&["EMAIL"]), vec![None, None, None])
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
            Chunk::new(TextLocation::inline(0..3), TextData::new("ccc".to_owned())),
            Chunk::new(TextLocation::inline(3..6), TextData::new("ddd".to_owned())),
        ]);
        let mut sources: Vec<&mut VecTextSource> = vec![&mut a, &mut c];

        let analyses = analyzer
            .analyze_streams(&mut sources, &scope_for(&["EMAIL"]), vec![None, None])
            .await
            .unwrap();

        // Source A (one chunk) has one finding; source C (two chunks) has two, the
        // second lifted to the second chunk's source offset.
        assert_eq!(analyses[0].entities.len(), 1);
        assert_eq!(analyses[1].entities.len(), 2);
        let ranges: Vec<_> = analyses[1]
            .entities
            .iter()
            .filter_map(|e| e.location().range().cloned())
            .collect();
        assert!(ranges.contains(&(0..3)));
        assert!(ranges.contains(&(3..6)));
    }

    /// What the layers and the catalog drop comes back with the analysis after
    /// each chunk's findings, suppressed, each trail naming its dropper, and
    /// lifted to its source's coordinates like the findings.
    #[tokio::test]
    async fn analyze_streams_carries_drops_in_source_coordinates() {
        let analysis = dropping_analyzer()
            .analyze_streams(
                &mut [&mut two_chunks()],
                &scope_for(&["EMAIL", "PHONE"]),
                vec![None],
            )
            .await
            .unwrap()
            .pop()
            .unwrap();

        let entities: Vec<_> = analysis
            .entities
            .iter()
            .map(|entity| {
                (
                    entity.label().as_str(),
                    entity.location().range().cloned().unwrap(),
                    entity
                        .audit()
                        .dropped()
                        .map(|dropped| dropped.by.name.as_str()),
                )
            })
            .collect();
        assert_eq!(
            entities,
            [
                ("EMAIL", 0..3, None),
                ("PHONE", 1..2, Some("threshold")),
                ("IBAN", 0..3, Some("catalog")),
                ("EMAIL", 3..6, None),
                ("PHONE", 4..5, Some("threshold")),
                ("IBAN", 3..6, Some("catalog")),
            ]
        );
        for entity in &analysis.entities {
            assert_eq!(entity.is_suppressed(), entity.is_dropped());
            assert!(entity.audit().verify().is_ok());
        }
    }

    /// An analyzer that discards its drops returns only the findings.
    #[tokio::test]
    async fn a_discarding_analyzer_returns_only_the_findings() {
        let analysis = dropping_analyzer()
            .discard_dropped()
            .analyze_streams(
                &mut [&mut two_chunks()],
                &scope_for(&["EMAIL", "PHONE"]),
                vec![None],
            )
            .await
            .unwrap()
            .pop()
            .unwrap();
        assert_eq!(analysis.entities.len(), 2, "one EMAIL per chunk");
        assert!(analysis.entities.iter().all(|entity| !entity.is_dropped()));
    }

    /// An analyzer that finds an EMAIL, an out-of-catalog IBAN over it, and a
    /// weak PHONE the threshold drops, on every chunk.
    fn dropping_analyzer() -> Analyzer<Text> {
        let weak = {
            let mut weak = detected("PHONE", (1, 2));
            weak.record(AuditEvent::calibrated(Confidence::new(0.2).unwrap(), 0.2));
            weak
        };
        Analyzer::<Text>::new()
            .with_recognizer(MockRecognizer::new(vec![
                detected("EMAIL", (0, 3)),
                detected("IBAN", (0, 3)),
                weak,
            ]))
            .with_layer(ThresholdLayer::default())
    }

    /// A source of two three-byte chunks.
    fn two_chunks() -> VecTextSource {
        VecTextSource(vec![
            Chunk::new(TextLocation::inline(0..3), TextData::new("ccc".to_owned())),
            Chunk::new(TextLocation::inline(3..6), TextData::new("ddd".to_owned())),
        ])
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
            .analyze_streams(&mut sources, &scope_for(&["EMAIL"]), vec![None])
            .await
            .expect_err("mismatched seed count is malformed input");
        assert_eq!(err.kind(), ErrorKind::MalformedInput);
    }

    /// A recognizer whose `recognize_batch` returns the wrong number of entity
    /// lists, violating the one-list-per-subject contract.
    #[derive(Component)]
    #[component(name = "miscounting", version = "1")]
    struct Miscounting;

    #[async_trait::async_trait]
    impl Recognizer<Text> for Miscounting {
        async fn recognize(
            &self,
            _subject: &Subject<Text>,
            _ctx: &Context<'_>,
        ) -> Result<Vec<Entity<Text>>> {
            Ok(Vec::new())
        }

        async fn recognize_batch(
            &self,
            _subjects: &[&Subject<Text>],
            _ctx: &Context<'_>,
        ) -> Result<Vec<Vec<Entity<Text>>>> {
            // One short, regardless of subject count.
            Ok(Vec::new())
        }
    }

    /// A registered recognizer that returns the wrong number of entity lists must
    /// error at the aggregation boundary, not let the transpose silently leave
    /// subjects under-attributed.
    #[tokio::test]
    async fn recognize_batch_rejects_a_recognizer_count_mismatch() {
        let analyzer = Analyzer::<Text>::new().with_recognizer(Miscounting);

        let mut a = VecTextSource::single("aaa");
        let mut sources: Vec<&mut VecTextSource> = vec![&mut a];

        let err = analyzer
            .analyze_streams(&mut sources, &scope_for(&["EMAIL"]), vec![None])
            .await
            .expect_err("a recognizer returning the wrong count is a contract violation");
        assert_eq!(err.kind(), ErrorKind::Recognition);
    }
}
