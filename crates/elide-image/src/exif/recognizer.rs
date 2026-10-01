//! [`ExifRecognizer`]: classifies an EXIF field chunk into an entity.

use elide_core::Result;
use elide_core::entity::Entity;
use elide_core::modality::metadata::Metadata;
use elide_core::primitive::ComponentId;
use elide_core::recognition::{Context, Recognizer, Subject};

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
    fn id(&self) -> ComponentId {
        ComponentId::new(SOURCE, env!("CARGO_PKG_VERSION"))
    }

    async fn recognize(
        &self,
        subject: &Subject<Metadata>,
        _ctx: &Context<'_, Metadata>,
    ) -> Result<Vec<Entity<Metadata>>> {
        let data = subject.data();
        Ok(label_for(data.key())
            .and_then(|label| Metadata::field_entity(data.key(), label, SOURCE))
            .into_iter()
            .collect())
    }
}
