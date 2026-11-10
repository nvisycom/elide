//! The built-in [`Metadata`] pipeline every orchestrator drives a document's
//! metadata through.
//!
//! A metadata field is detected by its format's label: the codec labels each
//! field it surfaces, and the field is flagged as an entity as the document's
//! hidden-content policy says. Nothing is registered for this; a caller's
//! [`Analyzer<Metadata>`] adds entities on top of the fields (its text
//! recognizers read each field's value, and what they find is a part of the
//! value), and a caller's
//! [`Anonymizer<Metadata>`] decides how the selected fields are treated; a
//! field its rules do not cover is erased. The default anonymizer erases every
//! entity, flagged or detected.

use elide_core::modality::metadata::Metadata;
use elide_detection::Analyzer;
use elide_redaction::operators::Erase;
use elide_redaction::{Anonymizer, Rule};

use super::ModalityPipeline;

/// The metadata pipeline every orchestrator starts with: no recognizer, and an
/// anonymizer that erases every selected field.
pub(crate) fn pipeline() -> ModalityPipeline<Metadata> {
    ModalityPipeline::new(
        Analyzer::new(),
        Anonymizer::new().with(Rule::fallback(Erase)),
    )
}
