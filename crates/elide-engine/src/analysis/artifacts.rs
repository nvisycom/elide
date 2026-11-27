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

use elide_core::entity::audit::AuditHash;
use elide_core::modality::Modality;
use serde_value::Value;

use super::group::{ArtifactGroup, ErasedGroup};
use crate::PartId;
use crate::wire::{Erased, digest, part_fields};

/// One group's enrichment artifact plus its modality routing, the artifact-side
/// mirror of a report entry.
///
/// The [`TypeId`] downcasts the artifact for a caller; the modality *name* tags
/// it on the wire (an artifact type is not tied to one modality, `Text` and
/// `Tabular` share `Tokens`, so the name cannot be recovered from the artifact
/// itself and is captured from `M::NAME` at insert time).
pub(crate) struct ArtifactEntry {
    pub(crate) modality: ModalityTag,
    pub(crate) artifact: Box<dyn ArtifactGroup>,
}

/// Which modality an artifact was produced for: its [`TypeId`], to downcast
/// the artifact, and its name, to tag it on the wire.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ModalityTag {
    pub(crate) id: TypeId,
    pub(crate) name: &'static str,
}

impl ModalityTag {
    /// The tag of modality `M`.
    pub(crate) fn of<M: Modality>() -> Self {
        Self {
            id: TypeId::of::<M>(),
            name: M::NAME,
        }
    }

    /// The tag of the modality `group` holds.
    pub(crate) fn of_group(group: &dyn ErasedGroup) -> Self {
        Self {
            id: group.modality(),
            name: group.modality_name(),
        }
    }
}

/// The enrichment an analysis produced: every enriched part's artifact, by
/// [`PartId`].
///
/// It is keyed exactly as the [`Report`] keys its parts: a named document's own
/// artifact is under its [`Document::id`](crate::Document::id).
///
/// Produced beside a [`Report`] by [`analyze`](crate::Orchestrator::analyze)
/// (see [`AnalyzedDocument`](crate::AnalyzedDocument)) and consumed by
/// [`re_analyze`](crate::Orchestrator::re_analyze). A part is in the set only
/// when it was enriched or restored, even to an empty artifact (an image OCR'd
/// to no text); a part of a modality with nothing to enrich (metadata, whose
/// artifact is [`NoArtifact`](elide_core::modality::NoArtifact)) has no
/// entry. An artifact whose type serializes to `null` is held in memory, but
/// is written as `null` and so reads back as no entry; the digest leaves it
/// out to match.
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

    /// Whether the set holds no part, as when it serializes to `parts: []`.
    ///
    /// A part whose artifact is empty (an image OCR'd to no text) still counts.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.parts.is_empty()
    }

    /// The enrichment [`artifact`](Modality::Artifact) of part `id`, as
    /// modality `P`.
    ///
    /// Read-only. `None` for an unknown part or a modality mismatch.
    pub fn part<P: Modality>(&self, id: &PartId) -> Option<&P::Artifact> {
        let entry = self.parts.get(id)?;
        if entry.modality.id != TypeId::of::<P>() {
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
        self.set_part(id, ModalityTag::of::<P>(), Box::new(artifact));
        self
    }

    /// Store an already-erased part artifact (the analyze and read-back paths).
    pub(crate) fn set_part(
        &mut self,
        id: PartId,
        modality: ModalityTag,
        artifact: Box<dyn ArtifactGroup>,
    ) {
        self.parts.insert(id, ArtifactEntry { modality, artifact });
    }

    /// A digest of the set, what a [`Restorer`](crate::Restorer) checks it
    /// against when it is read back: an altered layout would change what
    /// re-analysis reads.
    #[must_use]
    pub fn digest(&self) -> AuditHash {
        digest(self)
    }

    /// The fields the set's digest covers: each part by path, with its
    /// modality and its artifact.
    ///
    /// An artifact is covered by its serialized form, so the digest holds every
    /// field the set carries across a review gap, whatever its type. One that
    /// serializes to nothing (`null`) is left out, as reading the set back
    /// drops it: the set reads back with the digest it was written with.
    pub(crate) fn digest_fields(&self) -> Vec<Vec<u8>> {
        part_fields(self.parts.iter().filter_map(|(id, entry)| {
            // An artifact that cannot serialize cannot be written either; it is
            // still covered, by its error, so the digest never skips it.
            let bytes = match serde_value::to_value(Erased(entry.artifact.as_ref())) {
                Ok(Value::Unit | Value::Option(None)) => return None,
                Ok(value) => {
                    let mut bytes = Vec::new();
                    encode(&value, &mut bytes);
                    bytes
                }
                Err(error) => [&[u8::MAX], error.to_string().as_bytes()].concat(),
            };
            Some((id, entry.modality.name, vec![bytes]))
        }))
    }
}

/// Write `value` to `out` unambiguously: a tag byte per variant, and a length
/// before every string, byte string, sequence, and map. A map's entries are in
/// key order, so the same value always writes the same bytes.
fn encode(value: &Value, out: &mut Vec<u8>) {
    let len = |out: &mut Vec<u8>, len: usize| out.extend_from_slice(&(len as u64).to_le_bytes());
    match value {
        Value::Bool(v) => out.extend([0, u8::from(*v)]),
        Value::U8(v) => out.extend([1, *v]),
        Value::U16(v) => {
            out.push(2);
            out.extend_from_slice(&v.to_le_bytes());
        }
        Value::U32(v) => {
            out.push(3);
            out.extend_from_slice(&v.to_le_bytes());
        }
        Value::U64(v) => {
            out.push(4);
            out.extend_from_slice(&v.to_le_bytes());
        }
        Value::I8(v) => out.extend([5, v.to_le_bytes()[0]]),
        Value::I16(v) => {
            out.push(6);
            out.extend_from_slice(&v.to_le_bytes());
        }
        Value::I32(v) => {
            out.push(7);
            out.extend_from_slice(&v.to_le_bytes());
        }
        Value::I64(v) => {
            out.push(8);
            out.extend_from_slice(&v.to_le_bytes());
        }
        Value::F32(v) => {
            out.push(9);
            out.extend_from_slice(&v.to_bits().to_le_bytes());
        }
        Value::F64(v) => {
            out.push(10);
            out.extend_from_slice(&v.to_bits().to_le_bytes());
        }
        Value::Char(v) => {
            out.push(11);
            out.extend_from_slice(&u32::from(*v).to_le_bytes());
        }
        Value::String(v) => {
            out.push(12);
            len(out, v.len());
            out.extend_from_slice(v.as_bytes());
        }
        Value::Unit => out.push(13),
        Value::Option(None) => out.push(14),
        Value::Option(Some(v)) => {
            out.push(15);
            encode(v, out);
        }
        Value::Newtype(v) => {
            out.push(16);
            encode(v, out);
        }
        Value::Seq(items) => {
            out.push(17);
            len(out, items.len());
            for item in items {
                encode(item, out);
            }
        }
        Value::Map(entries) => {
            out.push(18);
            len(out, entries.len());
            for (key, value) in entries {
                encode(key, out);
                encode(value, out);
            }
        }
        Value::Bytes(v) => {
            out.push(19);
            len(out, v.len());
            out.extend_from_slice(v);
        }
    }
}

#[cfg(test)]
mod tests {
    use elide_core::modality::text::{Text, Tokens};

    use super::*;

    #[test]
    fn a_set_with_an_empty_artifact_is_not_empty() {
        assert!(ArtifactSet::new().is_empty());
        let set = ArtifactSet::new().insert_part::<Text>(PartId::leaf("a.txt"), Tokens::default());
        assert!(!set.is_empty());
    }
}
