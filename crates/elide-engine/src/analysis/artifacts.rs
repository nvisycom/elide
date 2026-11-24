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
use serde_value::Value;

use super::group::ArtifactGroup;
use crate::PartId;
use crate::wire::{Erased, part_fields};

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

    /// The fields the set's seal covers: each part by path, with its modality
    /// and its artifact.
    ///
    /// An artifact is covered by its serialized form, so the seal holds every
    /// field the set carries across a review gap, whatever its type. One that
    /// serializes to nothing (a modality with no enrichment) is left out, as
    /// it is never read back.
    pub(crate) fn seal_fields(&self) -> Vec<Vec<u8>> {
        part_fields(self.parts.iter().filter_map(|(id, entry)| {
            // An artifact that cannot serialize cannot be written either; it is
            // still covered, by its error, so the seal never skips it.
            let bytes = match serde_value::to_value(Erased(entry.artifact.as_ref())) {
                Ok(Value::Unit) => return None,
                Ok(value) => {
                    let mut bytes = Vec::new();
                    encode(&value, &mut bytes);
                    bytes
                }
                Err(error) => [&[u8::MAX], error.to_string().as_bytes()].concat(),
            };
            Some((id, entry.modality_name, vec![bytes]))
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
