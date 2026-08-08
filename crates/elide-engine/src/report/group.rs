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

/// A type-erased, downcastable group of entities (a `Vec<Entity<M>>`).
///
/// An implementation detail of the report's storage, surfaced only because
/// it appears as a bound (`Vec<Entity<M>>: EntityGroup`) on the
/// orchestrator's construction methods. Lets groups of different
/// modalities sit together while each stays recoverable by downcast; under
/// the `serde` feature it is additionally erased-serializable.
#[doc(hidden)]
pub trait EntityGroup: Send + Sync + MaybeErased {
    fn as_any(&self) -> &dyn Any;
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

/// A type-erased, downcastable group of operator selections (a
/// `Vec<Selection<M>>`).
///
/// The selection counterpart to [`EntityGroup`]: it lets the operator picks
/// for parts of different modalities sit together while each stays recoverable
/// by downcast, and — under the `serde` feature — erased-serializable, so a
/// review layer can serialize the picks for a whole document. Produced by the
/// orchestrator's per-part `select`, surfaced only because it appears in those
/// methods' return type.
#[doc(hidden)]
pub trait SelectionGroup: Send + Sync + MaybeErased {
    fn as_any(&self) -> &dyn Any;
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
