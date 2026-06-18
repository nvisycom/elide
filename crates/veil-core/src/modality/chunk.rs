//! [`Chunk<M>`]: one decoded unit of a streamed source.

use super::Modality;

/// One decoded unit yielded by a [`StreamDataReader`].
///
/// `data` is the per-modality wire payload; `location` is the coordinate
/// the source accepts in [`read_at`] / [`write_at`] to address the same
/// chunk again. `hints` carries out-of-band context strings the chunk's
/// structural neighbours surface — CSV/XLSX column headers, JSON object
/// keys, HTML parent text — for context-aware recognizers; sources
/// without such metadata leave it empty.
///
/// [`StreamDataReader`]: super::StreamDataReader
/// [`read_at`]: super::DataReader::read_at
/// [`write_at`]: super::DataWriter::write_at
#[derive(Debug, Clone, PartialEq)]
pub struct Chunk<M: Modality> {
    /// Coordinate addressing this chunk inside the source.
    pub location: M::Location,
    /// Wire payload at the chunk's location.
    pub data: M::Data,
    /// Out-of-band context strings recognizers should treat as
    /// in-context (column headers, parent element text, …). Empty when
    /// the source has no such metadata to surface.
    pub hints: Vec<String>,
}

impl<M: Modality> Chunk<M> {
    /// A chunk over `data` at `location`, with no context hints.
    pub fn new(location: M::Location, data: M::Data) -> Self {
        Self {
            location,
            data,
            hints: Vec::new(),
        }
    }

    /// Attach out-of-band context hint strings.
    #[must_use]
    pub fn with_hints(mut self, hints: Vec<String>) -> Self {
        self.hints = hints;
        self
    }
}
