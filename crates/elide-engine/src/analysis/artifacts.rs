//! [`ArtifactSet`]: the enrichment content a document's analysis produced,
//! keyed the same way as its [`Report`] but carrying *content* rather than
//! *references*.
//!
//! A [`Report`] is references only — each entity is a location into the body
//! plus its audit trail, adding no content of its own. Enrichment (an image's
//! OCR `Layout`, an audio clip's STT `Transcription`) is the opposite: it is
//! content *extracted from* the body, the very thing an entity's location points
//! into. Keeping it out of the report preserves that separation; it lives here,
//! in a parallel structure with the same body-and-parts-by-id shape.
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

/// One group's enrichment artifact plus its modality routing — the artifact-side
/// mirror of a report entry.
///
/// The [`TypeId`] downcasts the artifact for a caller; the modality *name* tags
/// it on the wire (an artifact type is not tied to one modality — `Text` and
/// `Tabular` share `Tokens` — so the name cannot be recovered from the artifact
/// itself and is captured from `M::NAME` at insert time).
pub(crate) struct ArtifactEntry {
    pub(crate) modality: TypeId,
    pub(crate) modality_name: &'static str,
    pub(crate) artifact: Box<dyn ArtifactGroup>,
}

/// The enrichment a document's analysis produced: the body's artifact and each
/// container part's, keyed by [`PartId`] exactly as the [`Report`] keys its
/// parts.
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
    pub(crate) body: Option<ArtifactEntry>,
    pub(crate) parts: HashMap<PartId, ArtifactEntry>,
}

impl ArtifactSet {
    /// An empty set — no body artifact, no parts.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The body's enrichment [`artifact`](Modality::Artifact) of modality `M`
    /// (the OCR `Layout`, STT `Transcription`, …), read-only. `None` when the
    /// body is a different modality, or the set has no body artifact.
    pub fn body<M: Modality>(&self) -> Option<&M::Artifact> {
        let entry = self.body.as_ref()?;
        if entry.modality != TypeId::of::<M>() {
            return None;
        }
        entry.artifact.as_any().downcast_ref::<M::Artifact>()
    }

    /// The enrichment [`artifact`](Modality::Artifact) of the container part
    /// `id`, as modality `P`, read-only. `None` for an unknown part or a
    /// modality mismatch.
    pub fn part<P: Modality>(&self, id: &PartId) -> Option<&P::Artifact> {
        let entry = self.parts.get(id)?;
        if entry.modality != TypeId::of::<P>() {
            return None;
        }
        entry.artifact.as_any().downcast_ref::<P::Artifact>()
    }

    /// Set the body's artifact, as modality `M`, replacing any already set.
    /// For rebuilding a set out of band (a review layer that persisted the
    /// artifacts separately).
    #[must_use]
    pub fn insert_body<M: Modality>(mut self, artifact: M::Artifact) -> Self
    where
        M::Artifact: ArtifactGroup,
    {
        self.body = Some(ArtifactEntry {
            modality: TypeId::of::<M>(),
            modality_name: M::NAME,
            artifact: Box::new(artifact),
        });
        self
    }

    /// Set the artifact of the container part `id`, as modality `P`, replacing
    /// any already set for that part.
    #[must_use]
    pub fn insert_part<P: Modality>(mut self, id: PartId, artifact: P::Artifact) -> Self
    where
        P::Artifact: ArtifactGroup,
    {
        self.parts.insert(
            id,
            ArtifactEntry {
                modality: TypeId::of::<P>(),
                modality_name: P::NAME,
                artifact: Box::new(artifact),
            },
        );
        self
    }

    /// Store an already-erased body artifact (the analyze path, which holds a
    /// `Box<dyn ArtifactGroup>` from the pipeline).
    pub(crate) fn set_body(
        &mut self,
        modality: TypeId,
        modality_name: &'static str,
        artifact: Box<dyn ArtifactGroup>,
    ) {
        self.body = Some(ArtifactEntry {
            modality,
            modality_name,
            artifact,
        });
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

impl serde::Serialize for ArtifactSet {
    /// Serialize to `{ body: {modality, artifact}?, parts: { id: {modality,
    /// artifact} } }`, mirroring the [`Report`](crate::Report)'s shape. `body`
    /// is null when there is no body artifact; a group whose artifact is empty
    /// (text/tabular, or un-enriched) is omitted entirely, so a document with no
    /// enrichment serializes to `{ body: null, parts: {} }`.
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;

        // `{ modality, artifact }` for one entry. The artifact serializes through
        // erasure; `modality` tags which `M::Artifact` to parse it back as.
        struct Group<'a>(&'a ArtifactEntry, &'a str);
        impl serde::Serialize for Group<'_> {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                struct Erased<'a>(&'a dyn ArtifactGroup);
                impl serde::Serialize for Erased<'_> {
                    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                        erased_serde::serialize(self.0, s)
                    }
                }
                let mut state = s.serialize_struct("ArtifactGroup", 2)?;
                state.serialize_field("modality", self.1)?;
                state.serialize_field("artifact", &Erased(self.0.artifact.as_ref()))?;
                state.end()
            }
        }

        // Every stored entry reaches the wire: the set only holds *enriched*
        // artifacts (an un-enriched payload or a no-enrichment modality is never
        // inserted), so even an empty one — an image OCR'd to no text — is
        // persisted, and a restored re-run reuses it rather than re-enriching.
        fn group(entry: &ArtifactEntry) -> Group<'_> {
            Group(entry, entry.modality_name)
        }

        let parts: HashMap<&str, Group<'_>> = self
            .parts
            .iter()
            .map(|(id, e)| (id.as_str(), group(e)))
            .collect();

        let mut state = serializer.serialize_struct("ArtifactSet", 2)?;
        state.serialize_field("body", &self.body.as_ref().map(group))?;
        state.serialize_field("parts", &parts)?;
        state.end()
    }
}
