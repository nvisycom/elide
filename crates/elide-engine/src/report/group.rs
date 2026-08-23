//! The type-erased, downcastable [`EntityGroup`] (a `Vec<Entity<M>>`) the
//! [`Report`] stores. It is `erased_serde::Serialize` so the whole report
//! serializes through erasure.
//!
//! [`Report`]: super::Report

use std::any::Any;

use elide_core::entity::Entity;
use elide_core::modality::Modality;

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
/// [`Report`]: super::Report
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
