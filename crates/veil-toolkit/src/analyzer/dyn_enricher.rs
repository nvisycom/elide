//! The private [`DynEnricher`] object-safe bridge over [`Enricher`].

use std::future::Future;
use std::pin::Pin;

use veil_core::Error;
use veil_core::modality::Modality;
use veil_core::recognition::{Enricher, RecognizerInput};

/// Object-safe bridge over [`Enricher`].
///
/// Core's [`Enricher::enrich`] returns `impl Future` (RPITIT), which is
/// not object-safe, so an ordered list of enrichers can't be stored as
/// `Arc<dyn Enricher<M>>`. This crate-private trait boxes the future so
/// the analyzer can hold trait objects; a blanket impl makes every
/// [`Enricher`] one automatically, so the boxing is invisible at the
/// public API.
pub(crate) trait DynEnricher<M: Modality>: Send + Sync {
    fn enrich_boxed<'a>(
        &'a self,
        input: &'a mut RecognizerInput<M>,
    ) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send + 'a>>;
}

impl<M, E> DynEnricher<M> for E
where
    M: Modality,
    E: Enricher<M>,
{
    fn enrich_boxed<'a>(
        &'a self,
        input: &'a mut RecognizerInput<M>,
    ) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send + 'a>> {
        Box::pin(self.enrich(input))
    }
}
