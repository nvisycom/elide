//! The type-erased, downcastable [`EntityGroup`] (a `Vec<Entity<M>>`) the
//! [`Report`] stores. It is `erased_serde::Serialize` so the whole report
//! serializes through erasure.
//!
//! [`Report`]: super::report::Report

use std::any::Any;

use elide_core::entity::Entity;
use elide_core::modality::{Modality, ModalityArtifact};

/// A type-erased, downcastable group of entities.
///
/// A document's body and its container parts span several modalities, so the
/// report holds each group erased and recovers it by downcast. This trait is
/// the erased face of that storage: it appears as the bound
/// `Vec<Entity<M>>: EntityGroup` on the orchestrator's construction methods, and
/// carries erased serialization ([`erased_serde::Serialize`]) so the whole
/// report serializes. Recover the concrete group with [`as_any`] /
/// [`as_any_mut`] and `downcast_ref`/`downcast_mut` to `Vec<Entity<M>>`.
///
/// [`Report`]: super::report::Report
/// [`as_any`]: Self::as_any
/// [`as_any_mut`]: Self::as_any_mut
pub trait EntityGroup: Send + Sync + erased_serde::Serialize {
    /// The group as `&dyn Any`, to `downcast_ref` to a `Vec<Entity<M>>`.
    fn as_any(&self) -> &dyn Any;
    /// The group as `&mut dyn Any`, to `downcast_mut` to a `Vec<Entity<M>>`.
    fn as_any_mut(&mut self) -> &mut dyn Any;
    /// The stable name of this group's modality ([`Modality::NAME`]) — the tag
    /// the serialized report carries so [`deserialize_report`] can route each
    /// group back to the right modality's parser.
    ///
    /// [`deserialize_report`]: crate::Orchestrator::deserialize_report
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

/// A type-erased, downcastable enrichment artifact ([`M::Artifact`]) the
/// [`Report`] stores beside each group's entities, so an image's OCR
/// `Layout` or an audio clip's `Transcription` survives the review gap and
/// a recognizer can re-run against it without re-enriching.
///
/// The artifact face of the erased storage, mirroring [`EntityGroup`]: it is
/// [`erased_serde::Serialize`] so it serializes through erasure, and reports
/// [`is_empty`](Self::is_empty) so an empty artifact (a text group's
/// [`NoArtifact`], or an un-enriched payload) is omitted from the wire.
///
/// The artifact carries **no** modality name of its own — an artifact type is
/// not tied to one modality (`Text` and `Tabular` both use `Tokens`), so it is
/// routed by the entry's modality (the sibling [`EntityGroup`]'s
/// [`modality_name`](EntityGroup::modality_name)), never by its own type.
///
/// [`M::Artifact`]: elide_core::modality::Modality::Artifact
/// [`Report`]: super::report::Report
/// [`NoArtifact`]: elide_core::modality::NoArtifact
pub trait ArtifactGroup: Send + Sync + erased_serde::Serialize {
    /// The artifact as `&dyn Any`, to `downcast_ref` to an `M::Artifact`.
    fn as_any(&self) -> &dyn Any;
    /// The artifact as `&mut dyn Any`, to `downcast_mut` to an `M::Artifact`.
    fn as_any_mut(&mut self) -> &mut dyn Any;
    /// Whether the artifact carries no enrichment — omitted from the wire.
    fn is_empty(&self) -> bool;
}

impl<A> ArtifactGroup for A
where
    A: ModalityArtifact + erased_serde::Serialize,
{
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn is_empty(&self) -> bool {
        ModalityArtifact::is_empty(self)
    }
}
