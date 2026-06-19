//! Shared test fixtures: re-exports the core [`Text`] modality and a
//! trivial in-memory [`DataReader`] over a single string, the way a
//! real text codec would slice values at byte ranges.
// A shared fixture exposes more than any one test uses.
#![allow(dead_code, unused_imports)]

use elide_core::Result;
use elide_core::modality::DataReader;
pub use elide_core::modality::text::{Text, TextData, TextLocation, TextReplacement};

/// An in-memory text source: reads the byte range of a location out of
/// one backing string.
pub struct TextSource(pub String);

impl TextSource {
    /// A source over `text`.
    pub fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }
}

impl DataReader<Text> for TextSource {
    async fn read_at(&self, location: &TextLocation) -> Result<Option<TextData>> {
        Ok(self.0.get(location.start..location.end).map(TextData::new))
    }
}
