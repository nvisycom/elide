//! [`StreamRecognizer`]: a recognizer that finds matches in the
//! recognized-text stream and returns stream-positioned [`EntityDraft`]s.

use std::future::Future;

use crate::Result;
use crate::modality::TextRecognizable;
use crate::recognition::{EntityDraft, RecognizerContext, RecognizerId};

/// Finds matches in the recognized-text stream, as stream-positioned drafts.
///
/// Returns [`EntityDraft`]s carrying a stream byte range, not yet placed in
/// the medium — the shared seam every text-localizable recognizer (regex /
/// dictionary, NER, LLM-over-text) produces, regardless of how it finds its
/// matches. A draft is then [`lift`]ed to a located [`Entity`]; wrapping the
/// recognizer in the enhancement crate's `Enhanced` adapter yields a full
/// [`Recognizer`]. Modalities whose text is a projection (image OCR, audio
/// STT) are supported because [`as_text`] reads their enrichment artifact.
///
/// `find` is `async` and fallible: a regex engine resolves synchronously and
/// infallibly (it returns `Ok`), but an NER or LLM backend awaits a model
/// call that can fail — and a failed model call must surface as an error, not
/// silently yield no matches (a redaction pipeline that drops PII on a
/// backend hiccup is worse than one that fails loudly).
///
/// [`lift`]: crate::recognition::lift
/// [`Entity`]: crate::entity::Entity
/// [`Recognizer`]: crate::recognition::Recognizer
/// [`as_text`]: crate::modality::TextRecognizable::as_text
pub trait StreamRecognizer<M: TextRecognizable>: Send + Sync {
    /// This recognizer's identity (name + version).
    fn id(&self) -> RecognizerId;

    /// Find matches in `text` (the recognized-text stream) and return them
    /// as stream-positioned drafts, or an error if the recognizer's backend
    /// failed.
    fn find(
        &self,
        text: &str,
        ctx: &RecognizerContext<'_, M>,
    ) -> impl Future<Output = Result<Vec<EntityDraft>>> + Send;
}
