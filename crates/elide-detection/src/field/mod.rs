//! [`FieldRecognizer`]: one entity per metadata field its format labels.

use elide_core::Result;
use elide_core::entity::Entity;
use elide_core::modality::metadata::Metadata;
use elide_core::primitive::ComponentId;
use elide_core::recognition::{Context, Recognizer, Subject};

/// Recognizes a metadata field by the label its codec stamped on it.
///
/// What a metadata field is (a location, a person, a device, a time) is fixed
/// by its format's schema, not inferred from its value, so the codec that
/// surfaces the field labels it ([`MetadataData::label`]) and this recognizer
/// turns the label into an entity. One recognizer serves every format, so a
/// metadata pipeline needs no per-format registration. A field its format does
/// not label yields nothing here.
///
/// [`MetadataData::label`]: elide_core::modality::metadata::MetadataData::label
#[derive(Debug, Default, Clone, Copy)]
pub struct FieldRecognizer;

#[async_trait::async_trait]
impl Recognizer<Metadata> for FieldRecognizer {
    fn id(&self) -> ComponentId {
        ComponentId::new(Metadata::FIELD_SOURCE, env!("CARGO_PKG_VERSION"))
    }

    async fn recognize(
        &self,
        subject: &Subject<Metadata>,
        _ctx: &Context<'_, Metadata>,
    ) -> Result<Vec<Entity<Metadata>>> {
        let data = subject.data();
        Ok(data
            .label()
            .and_then(|label| Metadata::field_entity(data.key(), label.clone()))
            .into_iter()
            .collect())
    }
}
