//! Type-erased, downcastable groups the [`Report`] stores: [`EntityGroup`]
//! (a `Vec<Entity<M>>`) and its selection counterpart [`SelectionGroup`] (a
//! `Vec<Selection<M>>`). `EntityGroup` carries the serde-conditional
//! [`MaybeErased`] capability so the whole report serializes; `SelectionGroup`
//! serializes instead through [`views`](SelectionGroup::views), since a
//! [`Selection`] holds a live operator.
//!
//! [`Report`]: super::Report
//! [`Selection`]: elide_redaction::Selection

use std::any::Any;

use elide_core::entity::Entity;
use elide_core::modality::Modality;
use elide_redaction::{Selection, SelectionView};

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
/// body and each container part span several modalities, so [`select`] hands
/// them back erased in a [`DocumentSelections`]. A `Selection` carries a live
/// operator and does not serialize, so the group is not serde-erased; instead
/// [`views`](Self::views) projects the whole group to `Vec<`[`SelectionView`]`>`
/// — the serializable, modality-free form a review layer displays or ships,
/// no downcast required. For the in-process apply path, recover the concrete
/// group with [`as_any`] / [`as_any_mut`] and `downcast_ref`/`downcast_mut` to
/// `Vec<Selection<M>>`.
///
/// [`select`]: crate::Orchestrator::select
/// [`DocumentSelections`]: super::DocumentSelections
/// [`as_any`]: Self::as_any
/// [`as_any_mut`]: Self::as_any_mut
pub trait SelectionGroup: Send + Sync {
    /// The group as `&dyn Any`, to `downcast_ref` to a `Vec<Selection<M>>`.
    fn as_any(&self) -> &dyn Any;
    /// The group as `&mut dyn Any`, to `downcast_mut` to a `Vec<Selection<M>>`.
    fn as_any_mut(&mut self) -> &mut dyn Any;
    /// The serializable, modality-free projection of every pick in the group.
    ///
    /// Works through erasure: each [`Selection`]'s [`view`](Selection::view)
    /// drops the live operator, so a review layer serializes the picks for a
    /// whole document without knowing any modality.
    fn views(&self) -> Vec<SelectionView>;
}

impl<M: Modality> SelectionGroup for Vec<Selection<M>> {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn views(&self) -> Vec<SelectionView> {
        self.iter().map(Selection::view).collect()
    }
}
