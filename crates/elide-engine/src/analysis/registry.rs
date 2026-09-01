//! The per-modality reconstruction registry driving deserialization: a group's
//! wire form drops its concrete modality type (tagged only by
//! [`Modality::NAME`]), and deserialization is not object-safe, so the concrete
//! type is recovered from a per-name parser registered here.
//!
//! The [`Orchestrator`] holds a [`ModalityRegistry`]: [`with_modality`] registers,
//! per modality name, a parser that deserializes that group as the concrete
//! `Vec<Entity<M>>` (and, alongside it, its `M::Artifact`). The seed visitors
//! that drive the buffered `{ body, parts }` traversal live in
//! [`serde`](super::serde); this module owns the registry those seeds route
//! through and the public [`ReportDeserializer`].
//!
//! [`Orchestrator`]: crate::Orchestrator
//! [`with_modality`]: crate::Orchestrator::with_modality
//! [`Modality::NAME`]: elide_core::modality::Modality::NAME

use std::any::TypeId;
use std::collections::HashMap;

use elide_core::entity::Entity;
use elide_core::modality::Modality;
use serde::de::{DeserializeSeed, Deserializer};

use super::artifacts::ArtifactSet;
use super::group::{ArtifactGroup, EntityGroup};
use super::report::Report;
use super::serde::{ArtifactSetSeed, ReportSeed};

/// Parses one erased entity group as its concrete `Vec<Entity<M>>`. Registered
/// per modality name by [`Orchestrator::with_modality`].
///
/// [`Orchestrator::with_modality`]: crate::Orchestrator::with_modality
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
/// group, and the routing [`TypeId`] the report entry stores for the apply path.
#[derive(Clone, Copy)]
pub(super) struct ModalityEntry {
    pub(super) parse: GroupParser,
    pub(super) parse_artifact: ArtifactParser,
    pub(super) type_id: TypeId,
    pub(super) modality_name: &'static str,
}

/// The orchestrator's per-modality reconstruction registry, keyed by
/// [`Modality::NAME`]. Populated by [`with_modality`] so a deserialized report
/// is reconstructed against exactly the modalities the orchestrator handles.
///
/// [`Modality::NAME`]: elide_core::modality::Modality::NAME
/// [`with_modality`]: crate::Orchestrator::with_modality
#[derive(Default)]
pub(crate) struct ModalityRegistry {
    entries: HashMap<&'static str, ModalityEntry>,
}

impl ModalityRegistry {
    /// Register modality `M`, keyed by its name. Called by
    /// [`with_modality`](crate::Orchestrator::with_modality) alongside the
    /// pipeline registration.
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
                modality_name: M::NAME,
            },
        );
    }

    /// The entry registered for `modality`, or `None` if this orchestrator has
    /// no pipeline for it.
    pub(super) fn entry(&self, modality: &str) -> Option<ModalityEntry> {
        self.entries.get(modality).copied()
    }

    /// Reconstruct a [`Report`] from `deserializer`, routing each group to its
    /// registered modality — the shared core of [`Report::deserializer`] and
    /// [`Orchestrator::deserialize_report`]. Maps any deserialization failure to
    /// a [`MalformedInput`] error.
    ///
    /// [`Report::deserializer`]: Report::deserializer
    /// [`Orchestrator::deserialize_report`]: crate::Orchestrator::deserialize_report
    /// [`MalformedInput`]: elide_core::ErrorKind::MalformedInput
    pub(crate) fn deserialize<'de, D>(&self, deserializer: D) -> elide_core::Result<Report>
    where
        D: Deserializer<'de>,
    {
        ReportSeed::new(self)
            .deserialize(deserializer)
            .map_err(|e| {
                elide_core::Error::new(elide_core::ErrorKind::MalformedInput, e.to_string())
            })
    }

    /// Reconstruct an [`ArtifactSet`](crate::ArtifactSet) from `deserializer`,
    /// routing each group's artifact to its registered modality's
    /// `parse_artifact`. Maps any failure to a [`MalformedInput`] error.
    ///
    /// [`MalformedInput`]: elide_core::ErrorKind::MalformedInput
    pub(crate) fn deserialize_artifacts<'de, D>(
        &self,
        deserializer: D,
    ) -> elide_core::Result<ArtifactSet>
    where
        D: Deserializer<'de>,
    {
        ArtifactSetSeed::new(self)
            .deserialize(deserializer)
            .map_err(|e| {
                elide_core::Error::new(elide_core::ErrorKind::MalformedInput, e.to_string())
            })
    }
}

/// Rebuilds a serialized [`Report`] — without the analyzers, anonymizers, or
/// codec registry needed to *run* one.
///
/// A report's wire form tags each group with its modality name but not the
/// concrete type, and deserialization is not object-safe, so a bare
/// `Report: Deserialize` is impossible. Register the modalities the report may
/// contain with [`with_modality`], then reconstruct with [`deserialize`]. The
/// modalities registered here need only be *deserializable*; they need no
/// pipeline.
///
/// This is the deserialize-only counterpart to the full [`Orchestrator`], for a
/// review layer that reconstructs an edited report without re-building the
/// engine that produced it. Build one with [`Report::deserializer`].
///
/// ```ignore
/// let report = Report::deserializer()
///     .with_modality::<Text>()
///     .with_modality::<Image>()
///     .deserialize(&mut serde_json::Deserializer::from_str(json))?;
/// ```
///
/// [`with_modality`]: Self::with_modality
/// [`deserialize`]: Self::deserialize
/// [`Orchestrator`]: crate::Orchestrator
/// [`Report::deserializer`]: Report::deserializer
#[derive(Default)]
pub struct ReportDeserializer {
    registry: ModalityRegistry,
}

impl ReportDeserializer {
    /// A deserializer with no modalities registered.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register modality `M` so a group tagged with its name reconstructs as
    /// `Vec<Entity<M>>`. Returns `self` for chaining.
    #[must_use]
    pub fn with_modality<M>(mut self) -> Self
    where
        M: Modality,
        Vec<Entity<M>>: serde::Serialize + serde::de::DeserializeOwned,
        M::Artifact: serde::Serialize + serde::de::DeserializeOwned,
    {
        self.registry.register::<M>();
        self
    }

    /// Reconstruct a [`Report`] from `deserializer`, routing each group to a
    /// registered modality.
    ///
    /// # Errors
    ///
    /// Returns a [`MalformedInput`] error if the payload is not a valid report,
    /// or if a group carries entities under a modality that was not registered —
    /// see [`Orchestrator::deserialize_report`] for the empty-vs-non-empty rule.
    ///
    /// [`MalformedInput`]: elide_core::ErrorKind::MalformedInput
    /// [`Orchestrator::deserialize_report`]: crate::Orchestrator::deserialize_report
    pub fn deserialize<'de, D>(&self, deserializer: D) -> elide_core::Result<Report>
    where
        D: Deserializer<'de>,
    {
        self.registry.deserialize(deserializer)
    }

    /// Reconstruct an [`ArtifactSet`](crate::ArtifactSet) from `deserializer`,
    /// routing each group's artifact to a registered modality. The
    /// artifact-side counterpart to [`deserialize`](Self::deserialize), for
    /// restoring the enrichment persisted beside a report.
    ///
    /// # Errors
    ///
    /// Returns a [`MalformedInput`] error if the payload is not a valid
    /// artifact set.
    ///
    /// [`MalformedInput`]: elide_core::ErrorKind::MalformedInput
    pub fn deserialize_artifacts<'de, D>(&self, deserializer: D) -> elide_core::Result<ArtifactSet>
    where
        D: Deserializer<'de>,
    {
        self.registry.deserialize_artifacts(deserializer)
    }
}
