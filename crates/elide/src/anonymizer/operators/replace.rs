//! [`Replace`]: substitute the matched span with a template string.

use elide_core::Result;
use elide_core::entity::Entity;
use elide_core::modality::Modality;
#[cfg(feature = "tabular")]
use elide_core::modality::tabular::{Tabular, TabularReplacement};
use elide_core::modality::text::{Text, TextData, TextReplacement};
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
    /// Identity shared by every modality's impl.
    fn id() -> OperatorId {
        OperatorId::new("replace", "1.0.0")
    }

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

impl Replace {
    /// Expand the template for an entity, substituting the placeholders.
    fn render<M: Modality>(&self, entity: &Entity<M>, value: &str) -> TextReplacement {
        let coref = entity.coref.as_ref().map_or("", |coref| coref.as_str());
        let rendered = self
            .template
            .replace("{label}", entity.label.as_str())
            .replace("{value}", value)
            .replace("{coref}", coref);
        TextReplacement::substituted(rendered)
    }
}

impl Operator<Text> for Replace {
    fn id(&self) -> OperatorId {
        Replace::id()
    }

    fn leak_profile(&self) -> LeakProfile {
        // Position and length of the rewritten span stay observable even
        // though the original value is gone.
        LeakProfile::Partial
    }

    async fn anonymize(&self, entity: &Entity<Text>, data: &TextData) -> Result<TextReplacement> {
        Ok(self.render(entity, data.as_str()))
    }
}

#[cfg(feature = "tabular")]
impl Operator<Tabular> for Replace {
    fn id(&self) -> OperatorId {
        Replace::id()
    }

    fn leak_profile(&self) -> LeakProfile {
        LeakProfile::Partial
    }

    async fn anonymize(
        &self,
        entity: &Entity<Tabular>,
        data: &TextData,
    ) -> Result<TabularReplacement> {
        Ok(self.render(entity, data.as_str()).into())
    }
}
