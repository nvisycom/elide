//! [`Replace`]: substitute the matched span with a template string.

use elide_core::entity::Entity;
use elide_core::modality::text::TextReplacement;
use elide_core::modality::{Modality, TextRedactable};
use elide_core::redaction::{LeakProfile, Operator};
use elide_core::{Component, Result};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Replaces an entity's text with a template string, like `[EMAIL]`.
///
/// Templates support three placeholders, expanded at apply time:
///
/// - `{label}`, the entity's label name (e.g. `PHONE_NUMBER`).
/// - `{value}`, the original matched substring.
/// - `{coref}`, the entity's coreference identifier, or empty when the
///   entity is not part of a resolved cluster.
///
/// The `{coref}` placeholder threads coreference through redaction: every
/// mention sharing an [`EntityCoRef`] expands to the same token, so a
/// template like `[{label}:{coref}]` renders `Alice`, `she`, and
/// `Ms. Smith` as one token while a distinct cluster stays distinct. The
/// substitution is pure: the identifier comes straight off the entity, so no
/// cross-call state is needed and the output is independent of the order
/// entities are processed.
///
/// A template with `{value}` writes the original back, and a coreference id
/// can embed the recognizer's handle for the cluster, which may echo a
/// mention, so a template with either reports [`LeakProfile::Recoverable`];
/// without them the profile is [`LeakProfile::Partial`].
///
/// The default template is `[{label}]`.
///
/// [`EntityCoRef`]: elide_core::entity::EntityCoRef
#[derive(Debug, Clone, Component)]
#[component(name = "replace", version = "1.0.0")]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Replace {
    template: String,
}

impl Replace {
    /// A [`Replace`] with the given template (see the type docs for
    /// placeholder syntax).
    pub fn new(template: impl Into<String>) -> Self {
        Self {
            template: template.into(),
        }
    }
}

impl Default for Replace {
    /// Default template `[{label}]`, a visible, label-tagged marker.
    fn default() -> Self {
        Self::new("[{label}]")
    }
}

/// The placeholder for the entity's label name.
const LABEL: &str = "{label}";
/// The placeholder for the original matched value.
const VALUE: &str = "{value}";
/// The placeholder for the entity's coreference id.
const COREF: &str = "{coref}";

impl Replace {
    /// Expand the template for an entity in one pass, so text a placeholder
    /// expands to is never expanded again.
    fn render<M: Modality>(&self, entity: &Entity<M>, value: &str) -> TextReplacement {
        let coref = entity.coref.as_ref().map_or("", |coref| coref.as_str());
        let placeholders = [
            (LABEL, entity.label.as_str()),
            (VALUE, value),
            (COREF, coref),
        ];
        let mut rendered = String::with_capacity(self.template.len());
        let mut rest = self.template.as_str();
        while let Some(open) = rest.find('{') {
            rendered.push_str(&rest[..open]);
            rest = &rest[open..];
            match placeholders
                .iter()
                .find(|(placeholder, _)| rest.starts_with(placeholder))
            {
                Some((placeholder, expansion)) => {
                    rendered.push_str(expansion);
                    rest = &rest[placeholder.len()..];
                }
                None => {
                    rendered.push('{');
                    rest = &rest[1..];
                }
            }
        }
        rendered.push_str(rest);
        TextReplacement::substituted(rendered)
    }
}

#[async_trait::async_trait]
impl<M: TextRedactable> Operator<M> for Replace {
    fn leak_profile(&self) -> LeakProfile {
        if self.template.contains(VALUE) || self.template.contains(COREF) {
            // `{value}` writes the original back, and a coreference id can
            // embed the recognizer's handle, which may echo the mention.
            LeakProfile::Recoverable
        } else {
            // Position and length of the rewritten span stay observable even
            // though the original value is gone.
            LeakProfile::Partial
        }
    }

    async fn anonymize(&self, entity: &Entity<M>, data: &M::Data) -> Result<M::Replacement> {
        Ok(M::Replacement::from(self.render(entity, data.as_ref())))
    }
}

#[cfg(test)]
mod tests {
    use elide_core::entity::EntityCoRef;
    use elide_core::modality::text::Text;

    use super::*;

    fn render(template: &str, value: &str) -> TextReplacement {
        let entity = Entity::<Text>::fixture("EMAIL", (0, value.len()))
            .with_coref(EntityCoRef::new("c1".to_owned()));
        Replace::new(template).render(&entity, value)
    }

    #[test]
    fn expands_every_placeholder() {
        assert_eq!(
            render("[{label}:{coref}] was {value}", "a@b.c"),
            TextReplacement::substituted("[EMAIL:c1] was a@b.c"),
        );
    }

    /// Text a placeholder expands to is not expanded again.
    #[test]
    fn expands_in_one_pass() {
        assert_eq!(
            render("{value}", "x{coref}{label}"),
            TextReplacement::substituted("x{coref}{label}"),
        );
        assert_eq!(
            render("{unknown}", "v"),
            TextReplacement::substituted("{unknown}")
        );
    }

    /// A template that can write the original back leaks it.
    #[test]
    fn a_value_or_coref_template_is_recoverable() {
        let profile = |template: &str| Operator::<Text>::leak_profile(&Replace::new(template));
        assert_eq!(profile("[{label}]"), LeakProfile::Partial);
        assert_eq!(profile("<{value}>"), LeakProfile::Recoverable);
        assert_eq!(profile("[{label}:{coref}]"), LeakProfile::Recoverable);
    }
}
