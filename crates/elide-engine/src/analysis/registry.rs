//! [`ReportDeserializer`]: the per-modality registry that rebuilds a serialized
//! [`Report`] or [`ArtifactSet`].
//!
//! A group's wire form drops its concrete modality type (tagged only by
//! [`Modality::NAME`]), and deserialization is not object-safe, so the concrete
//! type is recovered from a parser registered here per modality name. The seed
//! visitors that drive the buffered `{ parts: [..] }` traversal live in
//! [`serde`](super::serde); this module owns the parsers they route through.
//!
//! [`Modality::NAME`]: elide_core::modality::Modality::NAME

use std::any::TypeId;
use std::collections::HashMap;

use elide_core::entity::Entity;
use elide_core::modality::Modality;
use elide_core::{Error, Result};
use serde::de::{DeserializeSeed, Deserializer};

use super::artifacts::ArtifactSet;
use super::group::{ArtifactGroup, EntityGroup};
use super::report::Report;
use super::serde::{ArtifactSetSeed, ReportSeed};

/// Parses one erased entity group as its concrete `Vec<Entity<M>>`.
type GroupParser = fn(
    &mut dyn erased_serde::Deserializer<'_>,
) -> Result<Box<dyn EntityGroup>, erased_serde::Error>;

/// Parses one erased enrichment artifact as its concrete `M::Artifact`.
/// Registered alongside [`GroupParser`] so a group's artifact routes back to the
/// same modality as its entities.
type ArtifactParser = fn(
    &mut dyn erased_serde::Deserializer<'_>,
) -> Result<Box<dyn ArtifactGroup>, erased_serde::Error>;

/// What a registered modality contributes to reconstruction: how to parse its
/// entities and its artifact, and the modality's [`TypeId`], which the artifact
/// set stores for the apply path.
#[derive(Clone, Copy)]
pub(super) struct ModalityEntry {
    pub(super) parse: GroupParser,
    pub(super) parse_artifact: ArtifactParser,
    pub(super) type_id: TypeId,
}

/// Rebuilds a serialized [`Report`] or [`ArtifactSet`] against the modalities
/// registered with it.
///
/// A report's wire form tags each group with its modality name but not the
/// concrete type, and deserialization is not object-safe, so a bare
/// `Report: Deserialize` is impossible. Register the modalities the report may
/// contain with [`with_modality`], then reconstruct with [`deserialize`]. The
/// modalities registered here need only be *deserializable*; they need no
/// pipeline, so a review layer can rebuild an edited report without the engine
/// that produced it. Build one with [`Report::deserializer`], or use the one an
/// [`Orchestrator`] keeps for its registered modalities
/// ([`Orchestrator::deserializer`]).
///
/// ```
/// use elide_core::modality::metadata::Metadata;
/// use elide_core::modality::text::Text;
/// use elide_engine::Report;
///
/// let json = r#"{ "parts": [{ "id": ["notes.txt"], "modality": "text", "entities": [] }] }"#;
/// let report = Report::deserializer()
///     .with_modality::<Text>()
///     .with_modality::<Metadata>()
///     .deserialize(&mut serde_json::Deserializer::from_str(json))?;
/// assert_eq!(report.part_ids().count(), 1);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// [`with_modality`]: Self::with_modality
/// [`deserialize`]: Self::deserialize
/// [`Orchestrator`]: crate::Orchestrator
/// [`Orchestrator::deserializer`]: crate::Orchestrator::deserializer
/// [`Report::deserializer`]: Report::deserializer
#[derive(Default)]
pub struct ReportDeserializer {
    entries: HashMap<&'static str, ModalityEntry>,
}

impl ReportDeserializer {
    /// A deserializer with no modalities registered.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register modality `M`, so a group tagged with its name deserializes as
    /// it: its entities as `Vec<Entity<M>>`, its artifact as `M::Artifact`.
    #[must_use]
    pub fn with_modality<M>(mut self) -> Self
    where
        M: Modality,
        Vec<Entity<M>>: serde::Serialize + serde::de::DeserializeOwned,
        M::Artifact: serde::Serialize + serde::de::DeserializeOwned,
    {
        self.register::<M>();
        self
    }

    /// Register modality `M` in place, keyed by its name.
    pub(crate) fn register<M>(&mut self)
    where
        M: Modality,
        Vec<Entity<M>>: serde::Serialize + serde::de::DeserializeOwned,
        M::Artifact: serde::Serialize + serde::de::DeserializeOwned,
    {
        self.entries.insert(
            M::NAME,
            ModalityEntry {
                parse: |de| {
                    let entities: Vec<Entity<M>> = erased_serde::deserialize(de)?;
                    Ok(Box::new(entities) as Box<dyn EntityGroup>)
                },
                parse_artifact: |de| {
                    let artifact: M::Artifact = erased_serde::deserialize(de)?;
                    Ok(Box::new(artifact) as Box<dyn ArtifactGroup>)
                },
                type_id: TypeId::of::<M>(),
            },
        );
    }

    /// The registered name and entry for `modality`, or `None` if it is not
    /// registered.
    pub(super) fn entry(&self, modality: &str) -> Option<(&'static str, ModalityEntry)> {
        self.entries
            .get_key_value(modality)
            .map(|(&name, &entry)| (name, entry))
    }

    /// Reconstruct a [`Report`], routing each group to its registered modality.
    ///
    /// A group naming an unregistered modality is handled by what dropping it
    /// would cost: an *empty* group is skipped, since nothing is lost (as
    /// [`analyze`] ignores a part whose modality has no pipeline); a
    /// *non-empty* one is an error, since its entities may carry a reviewer's
    /// edits. A reconstructed report applies with [`anonymize_with`] exactly
    /// as one built by hand.
    ///
    /// # Errors
    ///
    /// [`MalformedInput`] if the payload is not a valid report, or if a group
    /// carries entities under an unregistered modality.
    ///
    /// [`analyze`]: crate::Orchestrator::analyze
    /// [`anonymize_with`]: crate::Orchestrator::anonymize_with
    /// [`MalformedInput`]: elide_core::ErrorKind::MalformedInput
    pub fn deserialize<'de, D>(&self, deserializer: D) -> Result<Report>
    where
        D: Deserializer<'de>,
    {
        ReportSeed::new(self)
            .deserialize(deserializer)
            .map_err(|e| Error::malformed(e.to_string()))
    }

    /// Reconstruct an [`ArtifactSet`], routing each group's artifact to its
    /// registered modality, to pass to [`re_analyze`] so the OCR/transcript is
    /// reused rather than recomputed.
    ///
    /// A group naming an unregistered modality is skipped: without a parser
    /// its enrichment cannot be rebuilt, and a re-run that lacks it simply
    /// re-enriches that group.
    ///
    /// # Errors
    ///
    /// [`MalformedInput`] if the payload is not a valid artifact set.
    ///
    /// [`re_analyze`]: crate::Orchestrator::re_analyze
    /// [`MalformedInput`]: elide_core::ErrorKind::MalformedInput
    pub fn deserialize_artifacts<'de, D>(&self, deserializer: D) -> Result<ArtifactSet>
    where
        D: Deserializer<'de>,
    {
        ArtifactSetSeed::new(self)
            .deserialize(deserializer)
            .map_err(|e| Error::malformed(e.to_string()))
    }
}
