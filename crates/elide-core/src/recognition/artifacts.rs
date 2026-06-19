//! [`Artifacts`] bundle: shared per-call NLP enrichment.

use std::fmt;

use type_map::concurrent::TypeMap;

/// Type-keyed bundle of shared per-call enrichment, carried on a
/// [`RecognizerContext`].
///
/// Upstream NLP work (tokenization, lemmatization, language detection)
/// is expensive and would be wasteful to repeat in every recognizer. A
/// caller that runs it once stashes the result here, keyed by its Rust
/// type, and any recognizer that wants it reads it back by type: e.g. a
/// context enhancer's lemma matcher pulls a `Tokens` artifact. Each type
/// has at most one entry. Recognizers that don't care leave it empty.
///
/// [`RecognizerContext`]: super::RecognizerContext
#[derive(Default)]
pub struct Artifacts(TypeMap);

impl Artifacts {
    /// Empty bundle.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert an artifact, replacing any previous value of the same type and
    /// returning it.
    pub fn insert<T: Send + Sync + 'static>(&mut self, artifact: T) -> Option<T> {
        self.0.insert(artifact)
    }

    /// Borrow the artifact of type `T`, if present.
    pub fn get<T: Send + Sync + 'static>(&self) -> Option<&T> {
        self.0.get::<T>()
    }

    /// Whether an artifact of type `T` is present.
    pub fn contains<T: Send + Sync + 'static>(&self) -> bool {
        self.0.contains::<T>()
    }
}

impl fmt::Debug for Artifacts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Artifacts").finish_non_exhaustive()
    }
}
