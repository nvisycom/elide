//! [`TextBacked`]: modalities whose payload and replacement are text.

use std::ops::Range;

use super::Modality;
use super::text::{Text, TextData, TextLocation, TextReplacement};

/// A modality whose per-chunk payload is text and whose redaction
/// replacement is a text replacement.
///
/// [`Text`] itself qualifies, and so does any modality that reuses
/// [`TextData`] / [`TextReplacement`] but addresses entities with its own
/// location type — notably `Tabular` (behind the `tabular` feature), where
/// a cell holds text. A recognizer or operator written for text serves any
/// of these unchanged: the only thing that varies is how a byte match
/// becomes that modality's *chunk-local* location, captured by [`locate`].
///
/// The chunk-local location built by `locate` carries only the byte range
/// within the chunk's text; the outer coordinates (a cell's row/column, a
/// page) are filled afterward by the codec's lift when streaming. So
/// `locate` states a truth — "a match spanning these bytes of this chunk"
/// — rather than fabricating a full source location.
///
/// [`locate`]: TextBacked::locate
pub trait TextBacked: Modality<Data = TextData, Replacement = TextReplacement> {
    /// Build a *chunk-local* location spanning `range` of the chunk's
    /// text. Outer coordinates default; lifting fills them in.
    fn locate(range: Range<usize>) -> Self::Location;

    /// The byte range a chunk-local `location` spans within the chunk's
    /// text — the inverse of [`locate`]. Used by post-recognition passes
    /// (keyword-boost enhancement) that re-read the matched text before
    /// the location is lifted to source coordinates.
    ///
    /// [`locate`]: TextBacked::locate
    fn span(location: &Self::Location) -> Range<usize>;
}

impl TextBacked for Text {
    fn locate(range: Range<usize>) -> TextLocation {
        TextLocation::new(range.start, range.end)
    }

    fn span(location: &TextLocation) -> Range<usize> {
        location.start..location.end
    }
}
