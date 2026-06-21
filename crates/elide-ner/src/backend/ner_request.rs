//! [`NerRequest`]: one per-call NER request handed to a [`NerBackend`].
//!
//! [`NerBackend`]: super::NerBackend

use elide_core::entity::Label;
use elide_core::primitive::LanguageTag;
use uuid::Uuid;

/// One per-call NER request handed to a [`NerBackend`].
///
/// [`NerBackend`]: super::NerBackend
#[derive(Debug, Clone)]
pub struct NerRequest<'a> {
    /// Source text to scan. Byte offsets in returned spans refer
    /// back into this string.
    pub text: &'a str,
    /// Labels to detect when the backend supports per-call label
    /// selection. `None` means the backend uses its built-in fixed
    /// label set; `Some(slice)` means restrict detection to the listed
    /// labels. Full [`Label`]s (name + optional description) rather than
    /// bare names: zero-shot models such as GLiNER 2.0 use the
    /// descriptions for better performance. Empty slice short-circuits
    /// the call to no work in the caller.
    ///
    /// [`Label`]: elide_core::entity::Label
    pub labels: Option<&'a [Label]>,
    /// Caller-asserted language. Backends that support per-call
    /// language hinting use this; backends that don't ignore it.
    pub language: Option<&'a LanguageTag>,
    /// Correlation UUID for tracing.
    pub correlation_id: Option<Uuid>,
}
