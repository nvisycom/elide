//! [`Restorer`]: the per-modality registry that rebuilds a serialized
//! [`Report`], [`Review`], or [`ArtifactSet`].
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
use serde::Serialize;
use serde::de::{DeserializeOwned, DeserializeSeed, Deserializer};

use super::seal::private::Sealed;
use super::seal::{Seal, SealKey, Sealable};
use super::serde::{ArtifactSetSeed, ReportSeed, ReviewSeed};
use crate::analysis::{ArtifactGroup, ArtifactSet, EntityGroup, Report};
use crate::review::{Edit, EditGroup, Review};

/// Parses one erased value as the concrete `T` a modality registered: its
/// entities, its review edits, or its artifact.
pub(super) type Parser<T> =
    fn(&mut dyn erased_serde::Deserializer<'_>) -> Result<T, erased_serde::Error>;

/// What a registered modality contributes to reconstruction: how to parse its
/// entities, its review edits, and its artifact, and the modality's
/// [`TypeId`], which the artifact set stores for the apply path.
#[derive(Clone, Copy)]
pub(super) struct ModalityEntry {
    pub(super) parse_entities: Parser<Box<dyn EntityGroup>>,
    pub(super) parse_edits: Parser<Box<dyn EditGroup>>,
    pub(super) parse_artifact: Parser<Box<dyn ArtifactGroup>>,
    pub(super) type_id: TypeId,
}

/// Rebuilds a serialized [`Report`], [`Review`], or [`ArtifactSet`] against
/// the modalities registered with it.
///
/// A report's wire form tags each group with its modality name but not the
/// concrete type, and deserialization is not object-safe, so a bare
/// `Report: Deserialize` is impossible. Register the modalities the report may
/// contain with [`with_modality`], [`seal`](Self::seal) a value when it is
/// written, then [`restore`] it against that [`Seal`]. The modalities registered
/// here need only be *deserializable*; they need no pipeline, so a review layer
/// can rebuild a report to show without the engine that produced it. Build one
/// with [`Report::restorer`], or use the one an [`Orchestrator`] keeps for
/// its registered modalities ([`Orchestrator::restorer`]).
///
/// ```
/// use elide_core::modality::metadata::Metadata;
/// use elide_core::modality::text::Text;
/// use elide_engine::Report;
///
/// let restorer = Report::restorer()
///     .with_modality::<Text>()
///     .with_modality::<Metadata>();
///
/// let written = Report::new();
/// let seal = restorer.seal(&written);
/// let json = serde_json::to_string(&written)?;
///
/// let report: Report = restorer.restore(&seal, &mut serde_json::Deserializer::from_str(&json))?;
/// assert_eq!(report.digest(), written.digest());
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// [`with_modality`]: Self::with_modality
/// [`restore`]: Self::restore
/// [`Orchestrator`]: crate::Orchestrator
/// [`Orchestrator::restorer`]: crate::Orchestrator::restorer
/// [`Report::restorer`]: Report::restorer
#[derive(Default)]
pub struct Restorer {
    entries: HashMap<&'static str, ModalityEntry>,
    key: Option<SealKey>,
}

impl Restorer {
    /// A restorer with no modalities registered.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Check seals with `key`, the [`SealKey`] they were made with: with a key
    /// set, only a keyed seal is accepted, so a carrier cannot replace one
    /// with an unkeyed digest of a value it changed. Without a key, only an
    /// unkeyed seal is accepted.
    #[must_use]
    pub fn with_seal_key(mut self, key: SealKey) -> Self {
        self.key = Some(key);
        self
    }

    /// Set the key keyed seals are checked with, in place.
    pub(crate) fn set_seal_key(&mut self, key: SealKey) {
        self.key = Some(key);
    }

    /// Register modality `M`, so a group tagged with its name deserializes as
    /// it: its entities as `Vec<Entity<M>>`, its artifact as `M::Artifact`.
    #[must_use]
    pub fn with_modality<M>(mut self) -> Self
    where
        M: Modality,
        M::Location: Serialize + DeserializeOwned,
        M::Artifact: Serialize + DeserializeOwned,
    {
        self.register::<M>();
        self
    }

    /// Register modality `M` in place, keyed by its name.
    pub(crate) fn register<M>(&mut self)
    where
        M: Modality,
        M::Location: Serialize + DeserializeOwned,
        M::Artifact: Serialize + DeserializeOwned,
    {
        self.entries.insert(
            M::NAME,
            ModalityEntry {
                parse_entities: |de| {
                    let entities: Vec<Entity<M>> = erased_serde::deserialize(de)?;
                    Ok(Box::new(entities) as Box<dyn EntityGroup>)
                },
                parse_edits: |de| {
                    let edits: Vec<Edit<M>> = erased_serde::deserialize(de)?;
                    Ok(Box::new(edits) as Box<dyn EditGroup>)
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

    /// The [`Seal`] to store beside `value` and [`restore`](Self::restore) it
    /// against later: keyed with the key set by
    /// [`with_seal_key`](Self::with_seal_key), unkeyed without one.
    ///
    /// Seal a value when it is written, after any [`pick`] or [`review`] that
    /// changes it.
    ///
    /// [`pick`]: crate::Orchestrator::pick
    /// [`review`]: crate::Orchestrator::review
    #[must_use]
    pub fn seal<T: Sealable>(&self, value: &T) -> Seal {
        Seal::new(value, self.key.as_ref())
    }

    /// Read a [`Report`], [`Review`], or [`ArtifactSet`] back and check it
    /// against `seal`, routing each part to its registered modality.
    ///
    /// For a report, every entity's audit trail must verify as well. With a
    /// key set by [`with_seal_key`](Self::with_seal_key), only a keyed seal
    /// is accepted; without one, only an unkeyed seal.
    ///
    /// A part naming an unregistered modality is handled by what dropping it
    /// would cost. An empty one is skipped, since nothing is lost (as
    /// [`analyze`] ignores a part whose modality has no pipeline). A report or
    /// review part that is not empty is an error, since its entities would go
    /// unredacted, or its edits unapplied. An artifact is skipped, which the
    /// seal then reports, so register every modality the writer had.
    ///
    /// # Errors
    ///
    /// [`MalformedInput`] if the payload is malformed, or if a part carries
    /// entities or edits under an unregistered modality; [`Integrity`] if a
    /// trail does not verify, the value does not match `seal`, or `seal` is
    /// unkeyed where a key is set; [`Configuration`] for a keyed seal with no
    /// key set.
    ///
    /// [`analyze`]: crate::Orchestrator::analyze
    /// [`MalformedInput`]: elide_core::ErrorKind::MalformedInput
    /// [`Integrity`]: elide_core::ErrorKind::Integrity
    /// [`Configuration`]: elide_core::ErrorKind::Configuration
    pub fn restore<'de, T, D>(&self, seal: &Seal, deserializer: D) -> Result<T>
    where
        T: Sealable,
        D: Deserializer<'de>,
    {
        let value = T::parse(self, deserializer)?;
        seal.check(&value, self.key.as_ref())?;
        Ok(value)
    }

    /// Read a [`Report`], [`Review`], or [`ArtifactSet`] back without checking
    /// it against a seal.
    ///
    /// A report's trails still verify, but nothing catches a trail cut back to
    /// an earlier event, an entity changed beside its trail, an entity or edit
    /// dropped, or an artifact altered. For a value only shown, or one that
    /// never left the side that reads it; read one that crossed to where its
    /// carrier could change it with [`restore`](Self::restore).
    ///
    /// # Errors
    ///
    /// As [`restore`](Self::restore), less the seal.
    pub fn restore_unsealed<'de, T, D>(&self, deserializer: D) -> Result<T>
    where
        T: Sealable,
        D: Deserializer<'de>,
    {
        T::parse(self, deserializer)
    }
}

/// Run `seed` over `deserializer`, a malformed payload an error of kind
/// [`MalformedInput`](elide_core::ErrorKind::MalformedInput).
fn parse<'de, S, D>(seed: S, deserializer: D) -> Result<S::Value>
where
    S: DeserializeSeed<'de>,
    D: Deserializer<'de>,
{
    seed.deserialize(deserializer)
        .map_err(|e| Error::malformed(e.to_string()))
}

impl Sealed for Report {
    const NAME: &'static str = "report";

    fn seal_fields(&self) -> Vec<Vec<u8>> {
        Report::seal_fields(self)
    }

    fn parse<'de, D: Deserializer<'de>>(registry: &Restorer, de: D) -> Result<Self> {
        parse(ReportSeed::new(registry), de)
    }
}

impl Sealed for Review {
    const NAME: &'static str = "review";

    fn seal_fields(&self) -> Vec<Vec<u8>> {
        Review::seal_fields(self)
    }

    fn parse<'de, D: Deserializer<'de>>(registry: &Restorer, de: D) -> Result<Self> {
        parse(ReviewSeed::new(registry), de)
    }
}

impl Sealed for ArtifactSet {
    const NAME: &'static str = "artifact set";

    fn seal_fields(&self) -> Vec<Vec<u8>> {
        ArtifactSet::seal_fields(self)
    }

    fn parse<'de, D: Deserializer<'de>>(registry: &Restorer, de: D) -> Result<Self> {
        parse(ArtifactSetSeed::new(registry), de)
    }
}
