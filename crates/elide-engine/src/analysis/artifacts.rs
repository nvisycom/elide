//! [`ArtifactSet`]: the enrichment content a document set's analysis produced,
//! keyed the same way as its [`Report`] but carrying *content* rather than
//! *references*.
//!
//! A [`Report`] is references only, each entity is a location into a part plus
//! its audit trail, adding no content of its own. Enrichment (an image's OCR
//! `Layout`, an audio clip's STT `Transcription`) is the opposite: it is content
//! *extracted from* a part, the very thing an entity's location points into.
//! Keeping it out of the report preserves that separation; it lives here, in a
//! parallel structure with the same parts-by-[`PartId`] shape, a named
//! document's own artifact is its depth-1 part.
//!
//! Its purpose is re-recognition after a time gap: persist the [`ArtifactSet`]
//! alongside the report, and [`re_analyze`](crate::Orchestrator::re_analyze)
//! seeds each group's recognition with its artifact so the OCR/transcript is
//! reused instead of recomputed.
//!
//! [`Report`]: crate::Report

use std::any::TypeId;
use std::collections::HashMap;

use elide_core::modality::Modality;

use super::group::ArtifactGroup;
use crate::PartId;

/// One group's enrichment artifact plus its modality routing, the artifact-side
/// mirror of a report entry.
///
/// The [`TypeId`] downcasts the artifact for a caller; the modality *name* tags
/// it on the wire (an artifact type is not tied to one modality, `Text` and
/// `Tabular` share `Tokens`, so the name cannot be recovered from the artifact
/// itself and is captured from `M::NAME` at insert time).
pub(crate) struct ArtifactEntry {
    pub(crate) modality: TypeId,
    pub(crate) modality_name: &'static str,
    pub(crate) artifact: Box<dyn ArtifactGroup>,
}

/// The enrichment an analysis produced: every part's artifact, by [`PartId`].
///
/// It is keyed exactly as the [`Report`] keys its parts: a named document's own
/// artifact is under its [`Document::id`](crate::Document::id).
///
/// Produced beside a [`Report`] by [`analyze`](crate::Orchestrator::analyze)
/// (see [`AnalyzedDocument`](crate::AnalyzedDocument)) and consumed by
/// [`re_analyze`](crate::Orchestrator::re_analyze). A modality with no
/// enrichment (text, tabular) contributes an empty artifact, omitted from the
/// serialized form.
///
/// [`Report`]: crate::Report
#[derive(Default)]
pub struct ArtifactSet {
    pub(crate) parts: HashMap<PartId, ArtifactEntry>,
}

impl ArtifactSet {
    /// An empty set, no parts.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The enrichment [`artifact`](Modality::Artifact) of part `id`, as
    /// modality `P`.
    ///
    /// Read-only. `None` for an unknown part or a modality mismatch.
    pub fn part<P: Modality>(&self, id: &PartId) -> Option<&P::Artifact> {
        let entry = self.parts.get(id)?;
        if entry.modality != TypeId::of::<P>() {
            return None;
        }
        entry.artifact.as_any().downcast_ref::<P::Artifact>()
    }

    /// Set the artifact of the part `id`, as modality `P`, replacing any already
    /// set for that part.
    #[must_use]
    pub fn insert_part<P: Modality>(mut self, id: PartId, artifact: P::Artifact) -> Self
    where
        P::Artifact: serde::Serialize,
    {
        self.set_part(id, TypeId::of::<P>(), P::NAME, Box::new(artifact));
        self
    }

    /// Store an already-erased part artifact (the analyze path).
    pub(crate) fn set_part(
        &mut self,
        id: PartId,
        modality: TypeId,
        modality_name: &'static str,
        artifact: Box<dyn ArtifactGroup>,
    ) {
        self.parts.insert(
            id,
            ArtifactEntry {
                modality,
                modality_name,
                artifact,
            },
        );
    }
}
