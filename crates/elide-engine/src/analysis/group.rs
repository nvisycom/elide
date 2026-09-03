//! The type-erased, downcastable [`EntityGroup`] (a `Vec<Entity<M>>`) and
//! [`ArtifactGroup`] (an [`M::Artifact`]) the [`Report`] and [`ArtifactSet`]
//! store, so a document's body and parts can span several modalities behind one
//! type. Both are crate-internal: the public construction bounds are expressed
//! via `serde::Serialize` (which the blanket impls satisfy), so neither trait is
//! named in a public signature, and a downstream modality's group qualifies
//! automatically while no other type can pose as one.
//!
//! [`M::Artifact`]: elide_core::modality::Modality::Artifact
//! [`Report`]: super::report::Report
//! [`ArtifactSet`]: super::artifacts::ArtifactSet

use std::any::Any;

use elide_core::entity::Entity;
use elide_core::modality::{Modality, ModalityArtifact};

/// A type-erased, downcastable group of entities, a `Vec<Entity<M>>` behind
/// one type.
///
/// A document's body and its container parts span several modalities, so the
/// report holds each group erased and recovers it by downcast. Implemented only
/// for `Vec<Entity<M>>` (blanket over every `M: Modality`), and
/// [`erased_serde::Serialize`] so the whole report serializes through erasure.
pub(crate) trait EntityGroup: Send + Sync + erased_serde::Serialize {
    /// The group as `&dyn Any`, to `downcast_ref` to a `Vec<Entity<M>>`.
    fn as_any(&self) -> &dyn Any;
    /// The group as `&mut dyn Any`, to `downcast_mut` to a `Vec<Entity<M>>`.
    fn as_any_mut(&mut self) -> &mut dyn Any;
    /// The stable name of this group's modality ([`Modality::NAME`]), the tag
    /// the serialized report carries so deserialization routes each group back
    /// to the right modality's parser.
    ///
    /// [`Modality::NAME`]: elide_core::modality::Modality::NAME
    fn modality_name(&self) -> &'static str;
}

impl<M: Modality> EntityGroup for Vec<Entity<M>>
where
    Vec<Entity<M>>: serde::Serialize,
{
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn modality_name(&self) -> &'static str {
        M::NAME
    }
}

/// A type-erased, downcastable enrichment artifact ([`M::Artifact`]), the
/// artifact face of the erased storage, mirroring [`EntityGroup`].
///
/// The [`ArtifactSet`] stores one beside each group's entities, so an image's
/// OCR `Layout` or an audio clip's `Transcription` survives the review gap and a
/// recognizer can re-run against it without re-enriching. Implemented only for a
/// [`ModalityArtifact`]. It carries **no** modality name of its own: an artifact
/// type is not tied to one modality (`Text` and `Tabular` both use `Tokens`), so
/// it is routed by the entry's modality, never by its own type.
///
/// [`M::Artifact`]: elide_core::modality::Modality::Artifact
/// [`ArtifactSet`]: super::artifacts::ArtifactSet
pub(crate) trait ArtifactGroup: Send + Sync + erased_serde::Serialize {
    /// The artifact as `&dyn Any`, to `downcast_ref` to an `M::Artifact`.
    fn as_any(&self) -> &dyn Any;
}

impl<A> ArtifactGroup for A
where
    A: ModalityArtifact + erased_serde::Serialize,
{
    fn as_any(&self) -> &dyn Any {
        self
    }
}
