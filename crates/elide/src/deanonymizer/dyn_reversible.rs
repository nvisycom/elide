//! The private [`DynReversible`] object-safe bridge over
//! [`ReversibleOperator`].

use std::future::Future;
use std::pin::Pin;

use elide_core::Result;
use elide_core::entity::Entity;
use elide_core::modality::Modality;
use elide_core::redaction::ReversibleOperator;

/// A boxed, pinned future recovering the original data (or `None`).
type RecoverFuture<'a, M> =
    Pin<Box<dyn Future<Output = Result<Option<<M as Modality>::Data>>> + Send + 'a>>;

/// Object-safe bridge over [`ReversibleOperator`].
///
/// Core's [`ReversibleOperator::deanonymize`] returns `impl Future`
/// (RPITIT), which is not object-safe, so the registry can't store
/// `Arc<dyn ReversibleOperator<M>>`. This crate-private trait boxes the
/// future; a blanket impl makes every [`ReversibleOperator`] one
/// automatically, so the boxing is invisible at the public API.
pub(crate) trait DynReversible<M: Modality>: Send + Sync {
    fn deanonymize_boxed<'a>(
        &'a self,
        entity: &'a Entity<M>,
        replacement: &'a M::Replacement,
    ) -> RecoverFuture<'a, M>;
}

impl<M, O> DynReversible<M> for O
where
    M: Modality,
    O: ReversibleOperator<M>,
{
    fn deanonymize_boxed<'a>(
        &'a self,
        entity: &'a Entity<M>,
        replacement: &'a M::Replacement,
    ) -> RecoverFuture<'a, M> {
        Box::pin(self.deanonymize(entity, replacement))
    }
}
