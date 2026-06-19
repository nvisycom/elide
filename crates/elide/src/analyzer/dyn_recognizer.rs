//! The private [`DynRecognizer`] object-safe bridge over [`Recognizer`].

use std::future::Future;
use std::pin::Pin;

use elide_core::Result;
use elide_core::entity::Entity;
use elide_core::modality::Modality;
use elide_core::recognition::{Recognizer, RecognizerContext};

/// Boxed future a [`DynRecognizer`] returns: the recognized entities (or
/// an error), erased so trait objects can hold it.
type RecognizeFuture<'a, M> = Pin<Box<dyn Future<Output = Result<Vec<Entity<M>>>> + Send + 'a>>;

/// Object-safe bridge over [`Recognizer`].
///
/// Core's [`Recognizer::recognize`] returns `impl Future` (RPITIT),
/// which is not object-safe, so a heterogeneous list of recognizers
/// can't be stored as `Arc<dyn Recognizer<M>>`. This crate-private trait
/// boxes the future so the registry can hold trait objects; a blanket
/// impl makes every [`Recognizer`] one automatically, so the boxing is
/// invisible at the public API — callers only ever deal in
/// [`Recognizer`].
pub(crate) trait DynRecognizer<M: Modality>: Send + Sync {
    fn recognize_boxed<'a>(
        &'a self,
        data: &'a M::Data,
        ctx: &'a RecognizerContext<'_, M>,
    ) -> RecognizeFuture<'a, M>;
}

impl<M, R> DynRecognizer<M> for R
where
    M: Modality,
    R: Recognizer<M>,
{
    fn recognize_boxed<'a>(
        &'a self,
        data: &'a M::Data,
        ctx: &'a RecognizerContext<'_, M>,
    ) -> RecognizeFuture<'a, M> {
        Box::pin(self.recognize(data, ctx))
    }
}
