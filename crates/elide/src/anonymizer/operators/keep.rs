//! [`Keep`]: pass the matched span through unchanged.

use elide_core::Result;
use elide_core::entity::Entity;
use elide_core::modality::text::{Text, TextData, TextReplacement};
use elide_core::redaction::{LeakProfile, Operator, OperatorId};

/// Pass the matched span through unchanged.
///
/// Useful in mixed policies — mask everything by default but keep, say,
/// currency amounts readable. The replacement records the original
/// value verbatim so the audit trail still has a row.
#[derive(Debug, Clone, Copy, Default)]
pub struct Keep;

impl Operator<Text> for Keep {
    fn id(&self) -> OperatorId {
        OperatorId::new("keep", "1.0.0")
    }

    fn leak_profile(&self) -> LeakProfile {
        // The original value is unchanged: strictly the most leaky.
        LeakProfile::Recoverable
    }

    async fn anonymize(&self, _entity: &Entity<Text>, data: &TextData) -> Result<TextReplacement> {
        Ok(TextReplacement::substituted(data.as_str()))
    }
}
