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
use elide_core::entity::audit::AuditHash;
use elide_core::modality::Modality;
use elide_core::{Error, Result};
use serde::Serialize;
use serde::de::{DeserializeOwned, DeserializeSeed, Deserializer};

use super::digest::private::Sealed;
use super::digest::{Restorable, digest};
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
/// contain with [`with_modality`], keep the report's [digest](Report::digest)
/// when it is written, then [`restore`] it against that digest. The modalities
/// registered
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
/// let digest = written.digest();
/// let json = serde_json::to_string(&written)?;
///
/// let report: Report = restorer.restore(&digest, &mut serde_json::Deserializer::from_str(&json))?;
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
}

impl Restorer {
    /// A restorer with no modalities registered.
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

    /// Read a [`Report`], [`Review`], or [`ArtifactSet`] back and check it
    /// against `expected`, the digest it had when it was written, routing each
    /// part to its registered modality.
    ///
    /// Each entity's trail verifies on its own, but whoever carries a report
    /// can still rewrite an entity whole, cut a trail back to an earlier event,
    /// change a label or location beside its trail, or drop an entity; a
    /// review can lose an edit on the way, and an artifact set can change what
    /// re-analysis reads. The digest catches each of these, as long as it is
    /// kept where the carrier cannot change it: on the side that wrote the
    /// value, in a write-once store, or signed with a key the deployment holds.
    /// For a report, every entity's audit trail must verify as well.
    ///
    /// A part naming an unregistered modality is handled by what dropping it
    /// would cost. An empty one is skipped, since nothing is lost (as
    /// [`analyze`] ignores a part whose modality has no pipeline). A report or
    /// review part that is not empty is an error, since its entities would go
    /// unredacted, or its edits unapplied. An artifact is skipped, which the
    /// digest then reports, so register every modality the writer had.
    ///
    /// # Errors
    ///
    /// [`MalformedInput`] if the payload is malformed: a part carries entities
    /// or edits under an unregistered modality, a trail does not verify, or an
    /// entity says what its trail does not; [`Integrity`] if the value's
    /// digest is not `expected`.
    ///
    /// [`analyze`]: crate::Orchestrator::analyze
    /// [`MalformedInput`]: elide_core::ErrorKind::MalformedInput
    /// [`Integrity`]: elide_core::ErrorKind::Integrity
    pub fn restore<'de, T, D>(&self, expected: &AuditHash, deserializer: D) -> Result<T>
    where
        T: Restorable,
        D: Deserializer<'de>,
    {
        let value = T::parse(self, deserializer)?;
        if digest(&value) != *expected {
            return Err(Error::integrity(format!(
                "the {} does not match its digest: it was changed after it was written",
                T::NAME
            )));
        }
        Ok(value)
    }

    /// Read a [`Report`], [`Review`], or [`ArtifactSet`] back without checking
    /// its digest.
    ///
    /// A report's trails still verify, and each entity must still agree with
    /// its trail, but nothing catches a trail cut back to an earlier event, an
    /// entity or edit dropped, or an artifact altered. For a value only shown, or one that
    /// never left the side that reads it; read one that crossed to where its
    /// carrier could change it with [`restore`](Self::restore).
    ///
    /// # Errors
    ///
    /// As [`restore`](Self::restore), less the digest.
    pub fn restore_unchecked<'de, T, D>(&self, deserializer: D) -> Result<T>
    where
        T: Restorable,
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

    fn digest_fields(&self) -> Vec<Vec<u8>> {
        Report::digest_fields(self)
    }

    fn parse<'de, D: Deserializer<'de>>(registry: &Restorer, de: D) -> Result<Self> {
        parse(ReportSeed::new(registry), de)
    }
}

impl Sealed for Review {
    const NAME: &'static str = "review";

    fn digest_fields(&self) -> Vec<Vec<u8>> {
        Review::digest_fields(self)
    }

    fn parse<'de, D: Deserializer<'de>>(registry: &Restorer, de: D) -> Result<Self> {
        parse(ReviewSeed::new(registry), de)
    }
}

impl Sealed for ArtifactSet {
    const NAME: &'static str = "artifact set";

    fn digest_fields(&self) -> Vec<Vec<u8>> {
        ArtifactSet::digest_fields(self)
    }

    fn parse<'de, D: Deserializer<'de>>(registry: &Restorer, de: D) -> Result<Self> {
        parse(ArtifactSetSeed::new(registry), de)
    }
}
