//! The built-in [`Metadata`] pipeline every orchestrator drives a document's
//! metadata through.
//!
//! A metadata field is detected by its format's schema: the codec labels each
//! field it surfaces, so every field is an entity carrying that label. Nothing
//! is registered for this; a caller's [`Analyzer<Metadata>`] adds entities on
//! top of the fields, and a caller's [`Anonymizer<Metadata>`] decides how the
//! selected fields are treated. The default anonymizer erases every one.

use elide_codec::TypedStream;
use elide_core::Result;
use elide_core::entity::Entity;
use elide_core::modality::metadata::Metadata;
use elide_core::modality::{ModalityLocation, StreamDataReader};
use elide_core::recognition::Scope;
use elide_core::recognition::annotation::Annotations;
use elide_detection::Analyzer;
use elide_operator::operators::Erase;
use elide_redaction::{Anonymizer, Rule};

use super::ModalityPipeline;

/// The metadata pipeline every orchestrator starts with: the schema detection
/// of [`fields`], no recognizer, and an anonymizer that erases every selected
/// field.
pub(crate) fn pipeline() -> ModalityPipeline<Metadata> {
    ModalityPipeline::new(
        Analyzer::new(),
        Anonymizer::new().with(Rule::fallback(Erase)),
    )
    .with_schema(fields)
}

/// One entity per field `stream` surfaces whose label the scope's catalog
/// declares and no exclusion covers.
fn fields(
    stream: &TypedStream<Metadata>,
    scope: &Scope,
    annotations: &Annotations<Metadata>,
) -> Result<Vec<Entity<Metadata>>> {
    let entities = stream
        .chunks()?
        .iter()
        .filter_map(|chunk| Metadata::field_entity(chunk.data.key(), chunk.data.label().clone()))
        .filter(|entity| {
            !annotations
                .exclusions
                .iter()
                .any(|exclusion| entity.location.overlaps(&exclusion.location))
        })
        .collect();
    Ok(scope.catalog.retain_declared(entities))
}
