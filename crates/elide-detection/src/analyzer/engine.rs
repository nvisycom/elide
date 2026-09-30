//! The one analysis engine: turn a batch of sources into one [`Analysis`] each,
//! coalescing enrichment across the whole batch.
//!
//! Every public entry (single payload, single stream, many streams) funnels
//! through [`analyze_sources`](Analyzer::analyze_sources). It drains each
//! source's chunks into subjects, enriches the whole flat batch in one pass (so
//! an OCR/STT enricher issues one `call_batch` across every subject), recognizes
//! each, and re-aggregates per source — lifting each chunk's entities back to
//! source coordinates and carrying the source's single enrichment artifact out.

use elide_core::Result;
use elide_core::entity::Entity;
use elide_core::modality::{Chunk, ModalityLocation, StreamDataReader};
use elide_core::recognition::annotation::{Annotations, Exclusion};
use elide_core::recognition::{Recognition, RecognizerContext, Scope, Subject};
use futures::future;

use super::Analyzer;
use super::analysis::Analysis;

impl<M: elide_core::modality::Modality> Analyzer<M> {
    /// The one analysis engine: analyze every `source` together and return one
    /// [`Analysis`] per source, in order.
    ///
    /// Each source is seeded from its paired `seeds` entry (a re-run's restored
    /// artifact, or `None` for a first pass). Every source's chunks become
    /// subjects in one flat batch, so enrichment coalesces across all of them; a
    /// text source contributes many subjects and a single-chunk image/audio source
    /// one — the same path, only the count differs. Each source's entities are
    /// lifted back through its own chunks and aggregated, and its single
    /// enrichment artifact (image/audio produce one; text none) is carried out.
    ///
    /// # Panics
    ///
    /// Debug-asserts `sources` and `seeds` have equal length.
    pub(super) async fn analyze_sources<S>(
        &self,
        sources: &mut [&mut S],
        scope: &Scope,
        annotations: &Annotations<M>,
        seeds: Vec<Option<M::Artifact>>,
    ) -> Result<Vec<Analysis<M>>>
    where
        S: StreamDataReader<M> + ?Sized,
    {
        debug_assert_eq!(sources.len(), seeds.len(), "one seed per source");
        let ctx = RecognizerContext::new(scope).with_annotations(annotations);

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

        // Recognize each subject and aggregate per source: lift its entities back
        // through its own chunk, and carry the source's one enrichment artifact.
        // Each source's analysis starts from its seed; `seeds` is retained
        // read-only for the single-artifact invariant.
        let mut analyses: Vec<Analysis<M>> = seeds.iter().cloned().map(Analysis::seeded).collect();
        for (mut subject, (source, chunk)) in subjects.into_iter().zip(origins) {
            let analysis = self.recognize_subject(&mut subject, &ctx).await?;
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
    pub(super) async fn analyze_subject(
        &self,
        subject: &mut Subject<M>,
        ctx: &RecognizerContext<'_, M>,
    ) -> Result<Analysis<M>> {
        for enricher in &self.enrichers {
            enricher.enrich(subject, ctx).await?;
        }
        self.recognize_subject(subject, ctx).await
    }

    /// Recognize over an already-enriched `subject`: run the recognizers, stamp
    /// languages, reduce, restrict to the catalog, apply exclusions, and carry the
    /// enrichment artifact out.
    async fn recognize_subject(
        &self,
        subject: &mut Subject<M>,
        ctx: &RecognizerContext<'_, M>,
    ) -> Result<Analysis<M>> {
        let mut entities = self.recognize(subject, ctx).await?;
        ctx.languages(subject).stamp(&mut entities);
        let reduced = self.reduce(entities);
        // Restrict the *output* to the requested catalog only after
        // reconciliation, so a strong out-of-catalog detection can subsume a
        // weak in-catalog one nested inside it before being culled itself.
        let in_catalog = ctx.catalog().retain_declared(reduced);
        let entities = Self::apply_exclusions(in_catalog, ctx.exclusions());
        // Carry the enrichment artifact out with the entities so it can be
        // persisted and restored for a re-run without re-enriching.
        Ok(Analysis::new(entities).with_artifact(subject.artifact().cloned()))
    }

    /// Run every recognizer over `subject` concurrently and collect their
    /// entities. The first error is returned (fail-fast).
    ///
    /// Recognizers borrow `data` and `ctx`, so they are joined in place rather
    /// than spawned onto the runtime.
    async fn recognize(
        &self,
        subject: &Subject<M>,
        ctx: &RecognizerContext<'_, M>,
    ) -> Result<Vec<Entity<M>>> {
        let futures = self
            .recognizers
            .iter()
            .map(|recognizer| recognizer.recognize(subject, ctx));
        let mut entities = Vec::new();
        for found in future::join_all(futures).await {
            let recognition: Recognition<M> = found?;
            entities.extend(recognition.entities);
        }
        Ok(entities)
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
