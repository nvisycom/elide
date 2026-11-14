//! [`NerRequest`]: one per-call NER request handed to a NER backend.

use elide_core::backend::Request;
use elide_core::entity::Label;
use elide_core::primitive::LanguageTag;
use uuid::Uuid;

/// One text handed to a NER backend, optionally with the labels to look for.
#[derive(Debug, Clone, Request)]
pub struct NerRequest<'a> {
    /// Source text to scan. Byte offsets in returned spans refer
    /// back into this string.
    pub text: &'a str,
    /// Labels to detect when the backend supports per-call label
    /// selection. `None` means the backend uses its built-in fixed
    /// label set; `Some(slice)` means restrict detection to the listed
    /// labels. Full [`Label`]s (name + optional description) rather than
    /// bare names: zero-shot models such as GLiNER 2.0 use the
    /// descriptions for better performance. An empty slice asks for no
    /// labels, so no spans: a backend answers it with an empty response
    /// without calling its model (see [`wants_nothing`](Self::wants_nothing)).
    ///
    /// [`Label`]: elide_core::entity::Label
    pub labels: Option<&'a [Label]>,
    /// Caller-asserted language. Backends that support per-call
    /// language hinting use this; backends that don't ignore it.
    #[backend(language)]
    pub language: Option<&'a LanguageTag>,
    /// Correlation UUID for tracing.
    pub correlation_id: Option<Uuid>,
}

impl NerRequest<'_> {
    /// Whether the request asks for no labels (`labels` is an empty slice),
    /// so its answer is no spans whatever the text.
    ///
    /// A backend checks this first and answers such a request with an empty
    /// response, without calling its model. `LabelScoped` forwards one when
    /// none of the requested labels is supported.
    #[must_use]
    pub fn wants_nothing(&self) -> bool {
        self.labels.is_some_and(<[Label]>::is_empty)
    }
}
