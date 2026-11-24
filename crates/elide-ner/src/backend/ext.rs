//! [`NerBackendExt`]: wrap a NER backend in a label decorator by method.

use elide_core::backend::Backend;
use elide_core::entity::LabelRef;

use crate::backend::{LabelExcluded, LabelScoped, NerRequest, NerResponse};

/// Wraps a NER backend in this crate's decorators, by method.
///
/// The methods apply in order, as `BackendExt` in `elide-backend` does for the
/// general ones:
///
/// ```ignore
/// let backend = gliner
///     .label_scoped([builtins::PERSON_NAME.to_ref(), builtins::ADDRESS.to_ref()])
///     .label_excluded([LabelRef::new("misc")]);
/// ```
///
/// Implemented for every NER backend.
pub trait NerBackendExt: Sized
where
    Self: for<'a> Backend<Request<'a> = NerRequest<'a>, Response = NerResponse>,
{
    /// Scope the model to the labels it supports; see [`LabelScoped`].
    fn label_scoped(self, labels: impl IntoIterator<Item = LabelRef>) -> LabelScoped<Self> {
        LabelScoped::new(self, labels)
    }

    /// Drop spans with any of `labels`; see [`LabelExcluded`].
    fn label_excluded(self, labels: impl IntoIterator<Item = LabelRef>) -> LabelExcluded<Self> {
        LabelExcluded::new(self, labels)
    }
}

impl<B> NerBackendExt for B where
    B: for<'a> Backend<Request<'a> = NerRequest<'a>, Response = NerResponse>
{
}
