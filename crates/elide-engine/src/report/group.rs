//! Type-erased, downcastable groups the [`Report`] stores: [`EntityGroup`]
//! (a `Vec<Entity<M>>`) and its selection counterpart [`SelectionGroup`] (a
//! `Vec<Selection<M>>`), plus the serde-conditional [`MaybeErased`] capability
//! they share.
//!
//! [`Report`]: super::Report

use std::any::Any;

use elide_core::entity::Entity;
use elide_core::modality::Modality;
use elide_redaction::Selection;

/// A type-erased, downcastable group of entities.
///
/// A document's body and its container parts span several modalities, so the
/// report holds each group erased and recovers it by downcast. This trait is
/// the erased face of that storage: it appears as the bound
/// `Vec<Entity<M>>: EntityGroup` on the orchestrator's construction methods,
/// and — under the `serde` feature — carries erased serialization so the whole
/// report serializes. Recover the concrete group with [`as_any`] /
/// [`as_any_mut`] and `downcast_ref`/`downcast_mut` to `Vec<Entity<M>>`.
///
/// [`Report`]: super::Report
/// [`as_any`]: Self::as_any
/// [`as_any_mut`]: Self::as_any_mut
pub trait EntityGroup: Send + Sync + MaybeErased {
    /// The group as `&dyn Any`, to `downcast_ref` to a `Vec<Entity<M>>`.
    fn as_any(&self) -> &dyn Any;
    /// The group as `&mut dyn Any`, to `downcast_mut` to a `Vec<Entity<M>>`.
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

impl<M: Modality> EntityGroup for Vec<Entity<M>>
where
    Vec<Entity<M>>: MaybeErased,
{
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// `MaybeErased` carries the serde-conditional capability in one place: it
// is `erased_serde::Serialize` with serde on, and a vacuous marker with it
// off. So `EntityGroup` and its construction sites need no `#[cfg]`.
#[cfg(feature = "serde")]
#[doc(hidden)]
pub use erased_serde::Serialize as MaybeErased;

#[cfg(not(feature = "serde"))]
#[doc(hidden)]
pub trait MaybeErased {}
#[cfg(not(feature = "serde"))]
impl<T> MaybeErased for T {}

/// A type-erased, downcastable group of selections.
///
/// The selection counterpart to [`EntityGroup`]: the operator picks for the
/// body and each container part span several modalities, so [`select_body`] /
/// [`select_part`] hand them back erased. Under the `serde` feature the group
/// is erased-serializable, so a review layer can serialize the picks for a
/// whole document. Recover the concrete group with [`as_any`] / [`as_any_mut`]
/// and `downcast_ref`/`downcast_mut` to `Vec<Selection<M>>`.
///
/// [`select_body`]: crate::Orchestrator::select_body
/// [`select_part`]: crate::Orchestrator::select_part
/// [`as_any`]: Self::as_any
/// [`as_any_mut`]: Self::as_any_mut
pub trait SelectionGroup: Send + Sync + MaybeErased {
    /// The group as `&dyn Any`, to `downcast_ref` to a `Vec<Selection<M>>`.
    fn as_any(&self) -> &dyn Any;
    /// The group as `&mut dyn Any`, to `downcast_mut` to a `Vec<Selection<M>>`.
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

impl<M: Modality> SelectionGroup for Vec<Selection<M>>
where
    Vec<Selection<M>>: MaybeErased,
{
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
