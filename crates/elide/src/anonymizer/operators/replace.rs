//! [`Replace`]: substitute the matched span with a template string.

use elide_core::Result;
use elide_core::entity::Entity;
use elide_core::modality::TextBacked;
use elide_core::modality::text::{TextData, TextReplacement};
use elide_core::redaction::{LeakProfile, Operator, OperatorId};

/// Substitute the matched span with a template string.
///
/// Templates support three placeholders, expanded at apply time:
///
/// - `{label}` — the entity's label name (e.g. `PHONE_NUMBER`).
/// - `{value}` — the original matched substring.
/// - `{coref}` — the entity's coreference identifier, or empty when the
///   entity is not part of a resolved cluster.
///
/// The `{coref}` placeholder threads coreference through redaction: every
/// mention sharing an [`EntityCoRef`] expands to the same token, so a
/// template like `[{label}:{coref}]` renders `Alice`, `she`, and
/// `Ms. Smith` all as `[PERSON:alice]` while a distinct cluster stays
/// distinct. The substitution is pure: the identifier comes straight off
/// the entity, so no cross-call state is needed and the output is
/// independent of the order entities are processed.
///
/// The default template is `[{label}]`.
///
/// [`EntityCoRef`]: elide_core::entity::EntityCoRef
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

impl<M: TextBacked> Operator<M> for Replace {
    fn id(&self) -> OperatorId {
        OperatorId::new("replace", "1.0.0")
    }

    fn leak_profile(&self) -> LeakProfile {
        // Position and length of the rewritten span stay observable even
        // though the original value is gone.
        LeakProfile::Partial
    }

    async fn anonymize(&self, entity: &Entity<M>, data: &TextData) -> Result<TextReplacement> {
        let coref = entity.coref.as_ref().map_or("", |coref| coref.as_str());
        let rendered = self
            .template
            .replace("{label}", entity.label.as_str())
            .replace("{value}", data.as_str())
            .replace("{coref}", coref);
        Ok(TextReplacement::substituted(rendered))
    }
}
