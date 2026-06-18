//! [`Replace`]: substitute the matched span with a template string.

use elide_core::Error;
use elide_core::entity::Entity;
use elide_core::modality::text::{Text, TextData, TextReplacement};
use elide_core::redaction::{LeakProfile, Operator, OperatorId};

/// Substitute the matched span with a template string.
///
/// Templates support two placeholders, expanded at apply time:
///
/// - `{label}` — the entity's label name (e.g. `PHONE_NUMBER`).
/// - `{value}` — the original matched substring.
///
/// The default template is `[{label}]`.
#[derive(Debug, Clone)]
pub struct Replace {
    template: String,
}

impl Replace {
    /// A `Replace` with the given template (see the type docs for
    /// placeholder syntax).
    pub fn new(template: impl Into<String>) -> Self {
        Self {
            template: template.into(),
        }
    }
}

impl Default for Replace {
    /// Default template `[{label}]` — a visible, label-tagged marker.
    fn default() -> Self {
        Self::new("[{label}]")
    }
}

impl Operator<Text> for Replace {
    fn id(&self) -> OperatorId {
        OperatorId::new("replace", "1.0.0")
    }

    fn leak_profile(&self) -> LeakProfile {
        // Position and length of the rewritten span stay observable even
        // though the original value is gone.
        LeakProfile::Partial
    }

    async fn anonymize(
        &self,
        entity: &Entity<Text>,
        data: &TextData,
    ) -> Result<TextReplacement, Error> {
        let rendered = self
            .template
            .replace("{label}", entity.label.as_str())
            .replace("{value}", data.as_str());
        Ok(TextReplacement::substituted(rendered))
    }
}
