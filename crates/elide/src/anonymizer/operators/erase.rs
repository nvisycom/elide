//! [`Erase`]: remove the matched span entirely.

use elide_core::Result;
use elide_core::entity::Entity;
use elide_core::modality::TextBacked;
use elide_core::modality::text::{TextData, TextReplacement};
use elide_core::redaction::{LeakProfile, Operator, OperatorId};

/// Remove the matched span entirely.
///
/// The codec writes nothing back at the entity's location; the span
/// disappears from the output. The strongest operator: no trace of the
/// original value or its shape remains. The name is modality-neutral, so
/// an image counterpart can clear a region and an audio one can cut an
/// interval under the same verb.
#[derive(Debug, Clone, Copy, Default)]
pub struct Erase;

impl<M: TextBacked> Operator<M> for Erase {
    fn id(&self) -> OperatorId {
        OperatorId::new("erase", "1.0.0")
    }

    fn leak_profile(&self) -> LeakProfile {
        LeakProfile::Irrecoverable
    }

    async fn anonymize(&self, _entity: &Entity<M>, _data: &TextData) -> Result<TextReplacement> {
        Ok(TextReplacement::Removed)
    }
}
