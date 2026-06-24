//! The private [`DynOperator`] object-safe bridge over [`Operator`].

use std::future::Future;
use std::pin::Pin;

use elide_core::Result;
use elide_core::entity::Entity;
use elide_core::modality::Modality;
use elide_core::redaction::{LeakProfile, Operator, OperatorId};

/// Object-safe bridge over [`Operator`].
///
/// Core's [`Operator::anonymize`] returns `impl Future` (RPITIT), which
/// is not object-safe, so a label→operator map can't store
/// `Arc<dyn Operator<M>>`. This crate-private trait boxes the future so
/// the registry can hold trait objects; a blanket impl makes every
/// [`Operator`] one automatically, so the boxing is invisible at the
/// public API — callers only ever deal in [`Operator`].
pub(crate) trait DynOperator<M: Modality>: Send + Sync {
    fn id(&self) -> OperatorId;

    fn leak_profile(&self) -> LeakProfile;

    fn anonymize_boxed<'a>(
        &'a self,
        entity: &'a Entity<M>,
        data: &'a M::Data,
    ) -> Pin<Box<dyn Future<Output = Result<M::Replacement>> + Send + 'a>>;
}

impl<M: Modality, O: Operator<M>> DynOperator<M> for O {
    fn id(&self) -> OperatorId {
        Operator::id(self)
    }

    fn leak_profile(&self) -> LeakProfile {
        Operator::leak_profile(self)
    }

    fn anonymize_boxed<'a>(
        &'a self,
        entity: &'a Entity<M>,
        data: &'a M::Data,
    ) -> Pin<Box<dyn Future<Output = Result<M::Replacement>> + Send + 'a>> {
        Box::pin(self.anonymize(entity, data))
    }
}
