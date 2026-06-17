//! The [`RecognizerRegistry`] — a per-modality store of recognizers,
//! run concurrently to detect entities.

use std::sync::Arc;

use tokio::task::JoinSet;
use type_map::concurrent::TypeMap;
use veil_core::Error;
use veil_core::entity::Entity;
use veil_core::modality::Modality;
use veil_core::recognition::{Recognizer, RecognizerInput, RecognizerOutput};

use super::dyn_recognizer::DynRecognizer;

/// One modality's slot in the registry: its list of recognizers.
struct Slot<M: Modality>(Vec<Arc<dyn DynRecognizer<M>>>);

/// A store of [`Recognizer`]s, keyed by modality.
///
/// Holds a separate list per [`Modality`] (so text recognizers and image
/// recognizers coexist in one registry without mixing). Running it
/// dispatches every recognizer for a modality concurrently and collects
/// their entities.
#[derive(Default)]
pub struct RecognizerRegistry {
    slots: TypeMap,
}

impl RecognizerRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a recognizer for its modality, returning the registry
    /// for chaining. Callers pass any [`Recognizer`]; the boxing for
    /// dynamic storage is internal.
    #[must_use]
    pub fn with_recognizer<M, R>(mut self, recognizer: R) -> Self
    where
        M: Modality + 'static,
        R: Recognizer<M> + 'static,
    {
        let recognizer: Arc<dyn DynRecognizer<M>> = Arc::new(recognizer);
        self.slots
            .entry::<Slot<M>>()
            .or_insert_with(|| Slot(Vec::new()))
            .0
            .push(recognizer);
        self
    }

    /// Run every recognizer registered for `M` over `input`,
    /// concurrently, and collect all recognized entities.
    ///
    /// Recognizers run in parallel; the first error aborts the rest and
    /// is returned (fail-fast). With no recognizers for the modality,
    /// returns an empty vec.
    pub async fn run<M>(&self, input: RecognizerInput<M>) -> Result<Vec<Entity<M>>, Error>
    where
        M: Modality + 'static,
    {
        let Some(slot) = self.slots.get::<Slot<M>>() else {
            return Ok(Vec::new());
        };

        let input = Arc::new(input);
        let mut set: JoinSet<Result<RecognizerOutput<M>, Error>> = JoinSet::new();
        for recognizer in &slot.0 {
            let recognizer = Arc::clone(recognizer);
            let input = Arc::clone(&input);
            set.spawn(async move { recognizer.recognize_boxed(&input).await });
        }

        let mut entities = Vec::new();
        while let Some(joined) = set.join_next().await {
            match joined {
                Ok(Ok(output)) => entities.extend(output.entities),
                Ok(Err(error)) => {
                    set.abort_all();
                    return Err(error);
                }
                Err(join) => {
                    set.abort_all();
                    return Err(Error::new(veil_core::ErrorKind::Recognition, join));
                }
            }
        }
        Ok(entities)
    }
}
