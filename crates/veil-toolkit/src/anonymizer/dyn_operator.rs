//! The private [`DynOperator`] object-safe bridge over [`Operator`].

use std::future::Future;
use std::pin::Pin;

use veil_core::Error;
use veil_core::entity::Entity;
use veil_core::modality::Modality;
use veil_core::redaction::Operator;

/// Object-safe bridge over [`Operator`].
///
/// Core's [`Operator::anonymize`] returns `impl Future` (RPITIT), which
/// is not object-safe, so a label→operator map can't store
/// `Arc<dyn Operator<M>>`. This crate-private trait boxes the future so
/// the registry can hold trait objects; a blanket impl makes every
/// [`Operator`] one automatically, so the boxing is invisible at the
/// public API — callers only ever deal in [`Operator`].
pub(crate) trait DynOperator<M: Modality>: Send + Sync {
    fn anonymize_boxed<'a>(
        &'a self,
        entity: &'a Entity<M>,
        data: &'a M::Data,
    ) -> Pin<Box<dyn Future<Output = Result<M::Replacement, Error>> + Send + 'a>>;
}

impl<M, O> DynOperator<M> for O
where
    M: Modality,
    O: Operator<M>,
{
    fn anonymize_boxed<'a>(
        &'a self,
        entity: &'a Entity<M>,
        data: &'a M::Data,
    ) -> Pin<Box<dyn Future<Output = Result<M::Replacement, Error>> + Send + 'a>> {
        Box::pin(self.anonymize(entity, data))
    }
}
