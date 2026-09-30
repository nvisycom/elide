//! The public analysis entry points, each a thin adapter onto the one engine
//! ([`analyze_sources`](Analyzer::analyze_sources)).
//!
//! A single in-memory payload, one streamed source, or many streamed sources are
//! all the same batch, differing only in size: `analyze*` is a batch of one
//! source with one chunk, `analyze_stream*` a batch of one source, and
//! `analyze_streams` a batch across sources.

use elide_core::Result;
use elide_core::modality::{Modality, StreamDataReader};
use elide_core::recognition::annotation::Annotations;
use elide_core::recognition::{RecognizerContext, Scope, Subject};

use super::Analyzer;
use super::analysis::Analysis;

impl<M: Modality> Analyzer<M> {
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
        let ctx = RecognizerContext::new(scope).with_annotations(annotations);
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
        Ok(analyses.pop().unwrap_or_default())
    }

    /// Analyze several streamed sources together, coalescing enrichment into one
    /// batched provider round-trip, and return one [`Analysis`] per source in
    /// order. Each source is paired with its prior enrichment `seeds` entry.
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
}
