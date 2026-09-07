//! [`ExifRecognizer`]: classifies an EXIF field chunk into an entity.

use elide_core::Result;
use elide_core::modality::metadata::{Metadata, MetadataData, field_entity};
use elide_core::recognition::{Recognition, Recognizer, RecognizerContext, RecognizerId};

use super::entity::{SOURCE, label_for};

/// Recognizes a privacy-relevant EXIF field.
///
/// A metadata chunk is one field; this classifies it by key through the shared
/// key/label table and, when it is one of the sensitive tags, emits a single
/// [`Entity`](elide_core::entity::Entity). A field the table does not recognize
/// yields nothing, so a benign tag is never surfaced.
#[derive(Debug, Default, Clone, Copy)]
pub struct ExifRecognizer;

#[async_trait::async_trait]
impl Recognizer<Metadata> for ExifRecognizer {
    fn id(&self) -> RecognizerId {
        RecognizerId::new(SOURCE, env!("CARGO_PKG_VERSION"))
    }

    async fn recognize(
        &self,
        data: &MetadataData,
        _ctx: &RecognizerContext<'_, Metadata>,
    ) -> Result<Recognition<Metadata>> {
        let entities = label_for(data.key())
            .and_then(|label| field_entity(data.key(), label, SOURCE))
            .into_iter()
            .collect();
        Ok(Recognition::new(entities))
    }
}
