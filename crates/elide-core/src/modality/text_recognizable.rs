//! [`TextRecognizable`]: text recognition over any modality.

use std::ops::Range;

use super::Modality;
use super::text::Token;

/// A modality whose content can be read as text for recognition, and that can
/// place a text match back into its own location coordinate space.
///
/// The text recognizers (pattern, NER) and the language enricher require two
/// things of a modality: a way to view its content as a string ([`as_text`]),
/// and a way to turn a byte range of that string into a modality location
/// ([`locate`]). It does **not** constrain the [`Replacement`] type, so a
/// modality that recognizes over text but redacts in its own medium qualifies.
///
/// Where the text lives differs by modality, so both methods receive the chunk
/// `data` *and* the medium's [`Artifact`](Modality::Artifact) and each draws
/// from the one it uses:
///
/// - [`Text`] and `Tabular` read their payload directly (it *is* [`TextData`]),
///   ignoring the artifact.
/// - Audio and image read text an enricher stamped onto the artifact (the
///   transcript, the OCR layout), ignoring the payload bytes.
///
/// [`as_text`]: TextRecognizable::as_text
/// [`locate`]: TextRecognizable::locate
/// [`Replacement`]: Modality::Replacement
/// [`Text`]: crate::modality::text::Text
/// [`TextData`]: crate::modality::text::TextData
pub trait TextRecognizable: Modality {
    /// The recognizable text a recognizer inspects, or [`None`] when the medium
    /// has none at this chunk.
    ///
    /// `Text` and `Tabular` always have text (their payload), so they return
    /// `Some` — even `Some("")` for a genuinely empty chunk. Audio and image
    /// return their enriched text (the transcript, the OCR layout) when the
    /// artifact is present, and [`None`] when it is not (not yet transcribed or
    /// OCR'd) so a recognizer skips the chunk rather than scan an empty string.
    fn as_text<'a>(data: &'a Self::Data, artifact: Option<&'a Self::Artifact>) -> Option<&'a str>;

    /// Place a match spanning `range` of the recognizable text into the medium's
    /// location, or `None` when the range cannot be placed.
    ///
    /// `Text` and `Tabular` keep the byte range as a *chunk-local* location and
    /// let a later lift fill the outer coordinates (a cell's row/column), so
    /// they always succeed. Audio and image resolve `range` against the artifact
    /// (the transcript's timings, the OCR layout) into a time span or pixel
    /// region, returning `None` when no enrichment covers it. A caller that gets
    /// `None` drops the match rather than emit an entity that addresses nowhere.
    fn locate(
        range: Range<usize>,
        data: &Self::Data,
        artifact: Option<&Self::Artifact>,
    ) -> Option<Self::Location>;

    /// The producer-provided [`Token`]s over the recognizable text, when the
    /// modality carries them.
    ///
    /// `Text` and `Tabular` tokenize into their [`Artifact`](Modality::Artifact),
    /// so their tokens — each with its lemma — feed lemma-aware context matching.
    /// Modalities that do not tokenize return the default `None`, and context
    /// matches on the surface text alone.
    fn as_tokens(_artifact: Option<&Self::Artifact>) -> Option<&[Token]> {
        None
    }
}
