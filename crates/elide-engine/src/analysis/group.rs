//! The type-erased, downcastable groups the [`Report`], the [`Review`], and the
//! [`ArtifactSet`] store, so a document set's parts can span several
//! modalities behind one type: an [`ErasedGroup`] of one modality's items
//! (an [`EntityGroup`] of entities, a review's edit group of edits), and an
//! [`ArtifactGroup`] (an [`M::Artifact`]). All are crate-internal: the public
//! construction bounds are expressed via `serde`, which the blanket impls
//! satisfy, so no trait is named in a public signature, and a downstream
//! modality's group qualifies automatically while no other type can pose as
//! one.
//!
//! [`M::Artifact`]: elide_core::modality::Modality::Artifact
//! [`Report`]: crate::Report
//! [`Review`]: crate::Review
//! [`ArtifactSet`]: crate::ArtifactSet

use std::any::{Any, TypeId};

use elide_core::entity::Entity;
use elide_core::entity::audit::AuditHash;
use elide_core::modality::{Modality, ModalityArtifact};
use serde::Serialize;
use serde::de::DeserializeOwned;
use uuid::Uuid;

/// One item of a group: something that belongs to a single modality, such as
/// an entity or a review edit.
pub(crate) trait GroupItem: Send + Sync + 'static {
    /// The modality the item belongs to.
    type Modality: Modality;
}

impl<M: Modality> GroupItem for Entity<M> {
    type Modality = M;
}

/// A type-erased, downcastable group of one modality's items, a `Vec<T>`
/// behind one type.
///
/// A document set's parts span several modalities, so each part's group is
/// held erased and recovered by downcast. Implemented for every `Vec` of a
/// [`GroupItem`] that serializes, so a group serializes through erasure.
pub(crate) trait ErasedGroup: Send + Sync + erased_serde::Serialize {
    /// The group as `&dyn Any`, to `downcast_ref` to its `Vec`.
    fn as_any(&self) -> &dyn Any;
    /// The group as `&mut dyn Any`, to `downcast_mut` to its `Vec`.
    fn as_any_mut(&mut self) -> &mut dyn Any;
    /// The group's modality, the key its pipeline is registered under.
    fn modality(&self) -> TypeId;
    /// The stable name of the group's modality ([`Modality::NAME`]), the tag
    /// a wire form carries so each group routes back to its modality's parser.
    fn modality_name(&self) -> &'static str;
    /// Whether the group holds no items.
    fn is_empty(&self) -> bool;
}

impl<T: GroupItem> ErasedGroup for Vec<T>
where
    Vec<T>: Serialize,
{
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn modality(&self) -> TypeId {
        TypeId::of::<T::Modality>()
    }

    fn modality_name(&self) -> &'static str {
        <T::Modality as Modality>::NAME
    }

    fn is_empty(&self) -> bool {
        Vec::is_empty(self)
    }
}

/// A group of one modality's entities, a `Vec<Entity<M>>` behind one type.
pub(crate) trait EntityGroup: ErasedGroup {
    /// Each entity's [`digest`](Entity::digest), ordered by entity id.
    fn digests(&self) -> Vec<AuditHash>;
    /// Each entity's id.
    fn ids(&self) -> Vec<Uuid>;
}

impl<M: Modality> EntityGroup for Vec<Entity<M>>
where
    M::Location: Serialize + DeserializeOwned,
{
    fn digests(&self) -> Vec<AuditHash> {
        let mut entities: Vec<&Entity<M>> = self.iter().collect();
        entities.sort_unstable_by_key(|entity| entity.id());
        entities.into_iter().map(Entity::digest).collect()
    }

    fn ids(&self) -> Vec<Uuid> {
        self.iter().map(Entity::id).collect()
    }
}

/// A type-erased, downcastable enrichment artifact ([`M::Artifact`]), the
/// artifact face of the erased storage.
///
/// The [`ArtifactSet`] stores one beside each group's entities, so an image's
/// OCR `Layout` or an audio clip's `Transcription` survives the review gap and a
/// recognizer can re-run against it without re-enriching. Implemented only for a
/// [`ModalityArtifact`]. It carries **no** modality name of its own: an artifact
/// type is not tied to one modality (`Text` and `Tabular` both use `Tokens`), so
/// it is routed by the entry's modality, never by its own type.
///
/// [`M::Artifact`]: elide_core::modality::Modality::Artifact
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
