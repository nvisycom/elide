//! [`TextRecognizable`] (text recognition over any modality) and
//! [`TextBacked`] (the subset that also redacts as text).

use std::ops::Range;

use super::Modality;
use super::text::{Text, TextData, TextLocation, TextReplacement};

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
/// *is* [`TextData`]). A medium whose payload is not itself text — audio,
/// whose recognizer reads a transcript carried alongside the bytes —
/// implements [`as_text`] to return that transcript and [`locate`] to
/// resolve the matched byte range into its native coordinate (a time span),
/// using the per-chunk `data` the method now receives.
///
/// [`as_text`]: TextRecognizable::as_text
/// [`locate`]: TextRecognizable::locate
/// [`Replacement`]: Modality::Replacement
pub trait TextRecognizable: Modality {
    /// View the chunk payload as the text a recognizer inspects.
    ///
    /// [`Text`] and `Tabular` return their payload string directly. A
    /// medium that carries text out of band (audio's transcript) returns
    /// that; when it has none, it returns `""`, so a recognizer simply
    /// finds nothing rather than erroring.
    fn as_text(data: &Self::Data) -> &str;

    /// Build the location of a match spanning `range` of the chunk text.
    ///
    /// For [`Text`] and `Tabular` the location is *chunk-local* — it carries
    /// the byte range and lifting fills the outer coordinates (a cell's
    /// row/column) later. For a medium whose location is not byte-based
    /// (audio time spans), `locate` resolves `range` against `data` (the
    /// transcript's timings) into the native coordinate immediately, so the
    /// emitted entity already addresses the source.
    fn locate(range: Range<usize>, data: &Self::Data) -> Self::Location;
}

/// A [`TextRecognizable`] modality whose payload and redaction replacement
/// are both text.
///
/// The subset of text-recognizable media that also *redact* as text:
/// [`Text`] itself, and `Tabular` (a cell holds text). The text operators
/// (`Erase`, `Mask`, …) and the keyword-boost enhancer bind here, because
/// they produce [`TextReplacement`]s and re-read the matched byte range via
/// [`span`]. A medium that recognizes over text but redacts in another
/// form (audio) is [`TextRecognizable`] but not `TextBacked`.
///
/// [`span`]: TextBacked::span
pub trait TextBacked: TextRecognizable<Data = TextData, Replacement = TextReplacement> {
    /// The byte range a chunk-local `location` spans within the chunk text
    /// — the inverse of [`locate`]. Used by post-recognition passes (the
    /// keyword-boost enhancer) that re-read the matched text before the
    /// location is lifted to source coordinates.
    ///
    /// [`locate`]: TextRecognizable::locate
    fn span(location: &Self::Location) -> Range<usize>;
}

impl TextRecognizable for Text {
    fn as_text(data: &TextData) -> &str {
        data.text.as_str()
    }

    fn locate(range: Range<usize>, _data: &TextData) -> TextLocation {
        TextLocation::new(range.start, range.end)
    }
}

impl TextBacked for Text {
    fn span(location: &TextLocation) -> Range<usize> {
        location.start..location.end
    }
}
