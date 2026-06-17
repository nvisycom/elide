//! [`Redact`]: delete the matched span entirely.

use veil_core::Error;
use veil_core::entity::Entity;
use veil_core::modality::text::{Text, TextData, TextReplacement};
use veil_core::redaction::{LeakProfile, Operator, OperatorId};

/// Delete the matched span entirely.
///
/// The codec writes nothing back at the entity's location; the span
/// disappears from the output. The strongest text operator — no trace
/// of the original value or its shape remains.
#[derive(Debug, Clone, Copy, Default)]
pub struct Redact;

impl Operator<Text> for Redact {
    fn id(&self) -> OperatorId {
        OperatorId::new("redact", "1.0.0")
    }

    fn leak_profile(&self) -> LeakProfile {
        LeakProfile::Irrecoverable
    }

    async fn anonymize(
        &self,
        _entity: &Entity<Text>,
        _data: &TextData,
    ) -> Result<TextReplacement, Error> {
        Ok(TextReplacement::Removed)
    }
}
