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

/// The enrichment a document set's analysis produced: every part's artifact
/// keyed by [`PartId`], exactly as the [`Report`] keys its parts, a named
/// document's own artifact is its depth-1 part.
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

    /// The **sole enriched document's** [`artifact`](Modality::Artifact) of
    /// modality `M` (the OCR `Layout`, STT `Transcription`, …), read-only, a
    /// shorthand for the single-document case. A set holds an entry only for a
    /// document that *produced* enrichment, so this returns the artifact when
    /// exactly one top-level document enriched; `None` when none did, when two or
    /// more did, or when that sole entry is a different modality than `M`. In a
    /// multi-document set, address a document's artifact by name with
    /// [`part`](Self::part) rather than relying on this shorthand.
    pub fn body<M: Modality>(&self) -> Option<&M::Artifact> {
        let entry = self.sole_document()?;
        if entry.modality != TypeId::of::<M>() {
            return None;
        }
        entry.artifact.as_any().downcast_ref::<M::Artifact>()
    }

    /// The enrichment [`artifact`](Modality::Artifact) of the part `id`, as
    /// modality `P`, read-only. `None` for an unknown part or a modality
    /// mismatch.
    pub fn part<P: Modality>(&self, id: &PartId) -> Option<&P::Artifact> {
        let entry = self.parts.get(id)?;
        if entry.modality != TypeId::of::<P>() {
            return None;
        }
        entry.artifact.as_any().downcast_ref::<P::Artifact>()
    }

    /// Set the **sole document's** artifact, as modality `M`, a shorthand for
    /// rebuilding a single-document set out of band (a review layer that
    /// persisted the artifacts separately). The document is keyed `name`.
    #[must_use]
    pub fn insert_body<M: Modality>(self, name: impl Into<PartId>, artifact: M::Artifact) -> Self
    where
        M::Artifact: serde::Serialize,
    {
        self.insert_part::<M>(name.into(), artifact)
    }

    /// Set the artifact of the part `id`, as modality `P`, replacing any already
    /// set for that part.
    #[must_use]
    pub fn insert_part<P: Modality>(mut self, id: PartId, artifact: P::Artifact) -> Self
    where
        P::Artifact: serde::Serialize,
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

    /// The single top-level (depth-1) *enriched* document's artifact entry, if
    /// the set holds exactly one, the backing for the [`body`](Self::body)
    /// single-document shorthand. `None` when no top-level document enriched or
    /// more than one did. A document that produced no enrichment has no entry
    /// here, so this counts enriched documents, not input documents.
    fn sole_document(&self) -> Option<&ArtifactEntry> {
        let mut tops = self.parts.iter().filter(|(id, _)| id.depth() == 1);
        let (_, only) = tops.next().filter(|_| tops.next().is_none())?;
        Some(only)
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
    /// Serialize to `{ parts: [ { id: [seg..], modality, artifact } ] }`,
    /// mirroring the [`Report`](crate::Report)'s shape. A group whose artifact is
    /// empty (text/tabular, or un-enriched) is never inserted, so a document with
    /// no enrichment serializes to `{ parts: [] }`. Each entry is keyed by its
    /// full [`PartId`] path.
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;

        // One entry: `{ id: [seg..], modality, artifact }`. The artifact
        // serializes through erasure; `modality` tags which `M::Artifact` to
        // parse it back as. The id rides as a segment array, never string-joined.
        struct Entry<'a>(&'a PartId, &'a ArtifactEntry);
        impl serde::Serialize for Entry<'_> {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                struct Path<'a>(&'a PartId);
                impl serde::Serialize for Path<'_> {
                    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                        s.collect_seq(self.0.segments())
                    }
                }
                struct Erased<'a>(&'a dyn ArtifactGroup);
                impl serde::Serialize for Erased<'_> {
                    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                        erased_serde::serialize(self.0, s)
                    }
                }
                let mut state = s.serialize_struct("ArtifactEntry", 3)?;
                state.serialize_field("id", &Path(self.0))?;
                state.serialize_field("modality", self.1.modality_name)?;
                state.serialize_field("artifact", &Erased(self.1.artifact.as_ref()))?;
                state.end()
            }
        }

        // Every stored entry reaches the wire: the set only holds *enriched*
        // artifacts (an un-enriched payload or a no-enrichment modality is never
        // inserted), so even an empty one, an image OCR'd to no text, is
        // persisted, and a restored re-run reuses it rather than re-enriching.
        let mut parts: Vec<Entry<'_>> = self.parts.iter().map(|(id, e)| Entry(id, e)).collect();
        // Deterministic wire output, matching the report serializer: sort by path
        // so a `HashMap`'s random iteration order does not churn the array.
        parts.sort_unstable_by(|a, b| a.0.segments().cmp(b.0.segments()));

        let mut state = serializer.serialize_struct("ArtifactSet", 1)?;
        state.serialize_field("parts", &parts)?;
        state.end()
    }
}
