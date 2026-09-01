//! [`TextRecognizable`]: text recognition over any modality.

use std::ops::Range;

use super::Modality;
use super::text::Token;

/// A modality whose per-chunk payload can be read as text for recognition,
/// and that can place a chunk-local text match into its own location
/// coordinate space.
///
/// This is what the text recognizers (pattern, NER, LLM) require: a way to
/// view the chunk payload as a string ([`as_text`]) and a way to turn a
/// byte range of that string into a modality location ([`locate`]). It does
/// **not** constrain the [`Replacement`] type, so a modality that recognizes
/// over text but redacts in its own medium qualifies.
///
/// [`Text`] and `Tabular` project their payload identically (their payload
/// *is* [`TextData`]). A medium whose recognizable text is not its payload —
/// audio, whose transcript an enricher stamps onto the call's
/// [`Artifact`](Modality::Artifact) — reads it from there instead. Both methods
/// receive the chunk `data` *and* the medium's `artifact` so each modality draws
/// from wherever its text and coordinate metadata live: text from `data`, audio
/// from the artifact.
///
/// [`as_text`]: TextRecognizable::as_text
/// [`locate`]: TextRecognizable::locate
/// [`Replacement`]: Modality::Replacement
/// [`Text`]: crate::modality::text::Text
/// [`TextData`]: crate::modality::text::TextData
pub trait TextRecognizable: Modality + Sized {
    /// View the recognizable text a recognizer inspects.
    ///
    /// `Text` and `Tabular` return their payload string from `data`, ignoring
    /// `artifact`. A medium whose text is enriched onto the call (audio's
    /// transcript) returns it from `artifact`; when `artifact` is [`None`] (not
    /// enriched) or empty it returns `""`, so a recognizer simply finds nothing
    /// rather than erroring.
    fn as_text<'a>(data: &'a Self::Data, artifact: Option<&'a Self::Artifact>) -> &'a str;

    /// Build the location of a match spanning `range` of the recognizable
    /// text, or `None` when the range cannot be placed in the medium.
    ///
    /// For `Text` and `Tabular` the location is *chunk-local* — it carries
    /// the byte range and lifting fills the outer coordinates (a cell's
    /// row/column) later — so it always succeeds. For a medium whose location
    /// is not byte-based (audio time spans, image regions), `locate` resolves
    /// `range` against `artifact` (the transcript's timings, the OCR layout)
    /// into the native coordinate, and returns `None` when no enrichment covers
    /// the range. A caller that gets `None` drops the match rather than emit an
    /// entity that addresses nowhere.
    fn locate(
        range: Range<usize>,
        data: &Self::Data,
        artifact: Option<&Self::Artifact>,
    ) -> Option<Self::Location>;

    /// The producer-provided [`Token`]s over the recognizable text, when the
    /// modality carries them — `Text` and `Tabular` tokenize into their
    /// [`Artifact`](Modality::Artifact), so their tokens (with each token's
    /// lemma) feed lemma-aware context matching. Modalities that do not
    /// tokenize return `None`, the default, and context matches on the surface
    /// text alone.
    fn as_tokens(_artifact: Option<&Self::Artifact>) -> Option<&[Token]> {
        None
    }
}
