//! The one-entity-list-per-subject contract of a batch recognizer, checked
//! wherever entity lists are paired back with the subjects they came from.

use elide_core::entity::Entity;
use elide_core::modality::Modality;
use elide_core::primitive::ComponentId;
use elide_core::{Error, Result};

/// Check that `recognizer` returned one entity list per input, the
/// [`Recognizer::recognize_batch`] contract, before the caller pairs the lists
/// with its inputs: a short or long result would otherwise drop findings or
/// index out of bounds.
///
/// # Errors
///
/// [`ErrorKind::Recognition`](elide_core::ErrorKind::Recognition) when the count
/// differs.
///
/// [`Recognizer::recognize_batch`]: elide_core::recognition::Recognizer::recognize_batch
pub(crate) fn expect_one_per_input<M: Modality>(
    recognizer: ComponentId,
    found: Vec<Vec<Entity<M>>>,
    inputs: usize,
) -> Result<Vec<Vec<Entity<M>>>> {
    if found.len() == inputs {
        return Ok(found);
    }
    Err(Error::recognition(format!(
        "recognizer {recognizer} returned {} entity lists for {inputs} inputs",
        found.len()
    )))
}
