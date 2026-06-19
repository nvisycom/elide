//! [`Redact`]: delete the matched span entirely.

use elide_core::Result;
use elide_core::entity::Entity;
use elide_core::modality::TextBacked;
use elide_core::modality::text::{TextData, TextReplacement};
use elide_core::redaction::{LeakProfile, Operator, OperatorId};

/// Delete the matched span entirely.
///
/// The codec writes nothing back at the entity's location; the span
/// disappears from the output. The strongest text operator — no trace
/// of the original value or its shape remains.
#[derive(Debug, Clone, Copy, Default)]
pub struct Redact;

impl<M: TextBacked> Operator<M> for Redact {
    fn id(&self) -> OperatorId {
        OperatorId::new("redact", "1.0.0")
    }

    fn leak_profile(&self) -> LeakProfile {
        LeakProfile::Irrecoverable
    }

    async fn anonymize(&self, _entity: &Entity<M>, _data: &TextData) -> Result<TextReplacement> {
        Ok(TextReplacement::Removed)
    }
}
