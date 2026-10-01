//! [`Analysis`]: the output of one analysis — the reconciled entities plus the
//! enrichment artifact they were produced from.

use elide_core::entity::Entity;
use elide_core::modality::Modality;

/// The output of one analysis: the reconciled entities and the enrichment
/// artifact.
#[derive(Debug, Clone)]
pub struct Analysis<M: Modality> {
    /// The reconciled entities, in the caller's coordinate system.
    pub entities: Vec<Entity<M>>,
    /// The enrichment artifact the analysis produced (or was seeded with): the
    /// OCR `Layout` / STT `Transcription` the recognizers read. Carried out
    /// so it can be persisted and restored for a re-run without re-enriching.
    ///
    /// [`Some`] iff an enricher ran (or a saved artifact was restored),
    /// `Some(empty)` (an image OCR'd to no text, a silent clip) is a real
    /// enrichment, distinct from [`None`] (a modality with no enrichment, or an
    /// un-enriched payload), so it is persisted and a re-run does not re-enrich.
    pub artifact: Option<M::Artifact>,
}

impl<M: Modality> Analysis<M> {
    /// An analysis carrying `entities` and no artifact.
    pub fn new(entities: Vec<Entity<M>>) -> Self {
        Self {
            entities,
            artifact: None,
        }
    }

    /// An empty analysis carrying only `artifact`: no entities, but the seeded
    /// enrichment carried through.
    ///
    /// The result for an input the scope asked nothing of (an empty catalog
    /// detects nothing) or a source with no chunk: a re-run seeded with a prior
    /// OCR/transcript must report it back unchanged so the run persists it, or the
    /// next re-run would re-enrich.
    pub fn seeded(artifact: Option<M::Artifact>) -> Self {
        Self::new(Vec::new()).with_artifact(artifact)
    }

    /// Attach the enrichment [`artifact`](Self::artifact) the analysis produced,
    /// `Some` when it enriched (even to an empty artifact), `None` otherwise.
    #[must_use]
    pub fn with_artifact(mut self, artifact: Option<M::Artifact>) -> Self {
        self.artifact = artifact;
        self
    }
}

impl<M: Modality> Default for Analysis<M> {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}
