//! The private [`DynRecognizer`] object-safe bridge over [`Recognizer`].

use std::future::Future;
use std::pin::Pin;

use elide_core::Error;
use elide_core::modality::Modality;
use elide_core::recognition::{Recognizer, RecognizerInput, RecognizerOutput};

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
        input: &'a RecognizerInput<M>,
    ) -> Pin<Box<dyn Future<Output = Result<RecognizerOutput<M>, Error>> + Send + 'a>>;
}

impl<M, R> DynRecognizer<M> for R
where
    M: Modality,
    R: Recognizer<M>,
{
    fn recognize_boxed<'a>(
        &'a self,
        input: &'a RecognizerInput<M>,
    ) -> Pin<Box<dyn Future<Output = Result<RecognizerOutput<M>, Error>> + Send + 'a>> {
        Box::pin(self.recognize(input))
    }
}
