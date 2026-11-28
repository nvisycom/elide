//! Filtering: dropping entities outright, below a confidence threshold or
//! outside the requested catalog.

use elide_core::Component;
use elide_core::entity::Entity;
use elide_core::modality::Modality;
use elide_core::primitive::ConfidenceThreshold;
use elide_core::recognition::Context;

use super::{Layer, LayerOutput};

/// The filtering stage: drop entities below a confidence threshold.
///
/// Which labels are detected is the request's catalog, which the analyzer
/// applies to its output with a [`CatalogFilter`]; this stage only drops the
/// weak findings.
#[derive(Debug, Clone, Copy, Component)]
#[component(name = "threshold")]
pub struct ThresholdLayer {
    threshold: ConfidenceThreshold,
}

impl ThresholdLayer {
    /// A filter dropping entities below `threshold`.
    #[must_use]
    pub fn new(threshold: ConfidenceThreshold) -> Self {
        Self { threshold }
    }

    /// Whether `entity` clears the threshold.
    pub fn passes<M: Modality>(&self, entity: &Entity<M>) -> bool {
        self.threshold.passes(entity.confidence())
    }
}

impl Default for ThresholdLayer {
    /// A filter at [`ConfidenceThreshold::BASELINE`].
    fn default() -> Self {
        Self::new(ConfidenceThreshold::BASELINE)
    }
}

impl<M: Modality> Layer<M> for ThresholdLayer {
    fn apply(&self, entities: Vec<Entity<M>>, _ctx: &Context<'_>) -> LayerOutput<M> {
        let mut output = LayerOutput::new();
        for entity in entities {
            if self.passes(&entity) {
                output.keep(entity);
            } else {
                output.reject(entity);
            }
        }
        output
    }
}

/// The output restriction: drop entities whose label the request's catalog
/// does not declare.
///
/// The analyzer runs it after every other layer, so a strong out-of-catalog
/// finding can still subsume a weak in-catalog one during reconciliation before
/// it is culled itself.
#[derive(Debug, Clone, Copy, Default, Component)]
#[component(name = "catalog")]
pub struct CatalogFilter;

impl<M: Modality> Layer<M> for CatalogFilter {
    fn apply(&self, entities: Vec<Entity<M>>, ctx: &Context<'_>) -> LayerOutput<M> {
        let mut output = LayerOutput::new();
        for entity in entities {
            if ctx.catalog().contains(entity.label()) {
                output.keep(entity);
            } else {
                output.reject(entity);
            }
        }
        output
    }
}

#[cfg(test)]
mod tests {
    use elide_core::entity::audit::{Pattern, PatternRule};
    use elide_core::entity::{Birth, Label, LabelCatalog, LabelRef};
    use elide_core::modality::text::{Text, TextLocation};
    use elide_core::primitive::{ComponentId, Confidence};
    use elide_core::recognition::Scope;

    use super::*;
    use crate::layer::Run;

    fn entity(label: &str, confidence: f32) -> Entity<Text> {
        Entity::new(Birth::new(
            Confidence::new(confidence).unwrap(),
            Pattern::new(
                ComponentId::new("t", "1"),
                LabelRef::new(label),
                TextLocation::inline(0..1),
                PatternRule::default(),
            ),
        ))
    }

    /// A candidate below the threshold is rejected outright.
    #[test]
    fn a_weak_candidate_is_rejected_by_the_threshold() {
        let layer = ThresholdLayer::new(ConfidenceThreshold::new(0.9).unwrap());
        let out = layer.run(vec![entity("ssn", 0.95), entity("ssn", 0.7)]);

        assert_eq!(out.kept().len(), 1);
        let [(dropped, winner)] = out.dropped() else {
            panic!("one drop: {:?}", out.dropped());
        };
        assert_eq!(dropped.confidence(), Confidence::new(0.7).unwrap());
        assert_eq!(*winner, None);
    }

    /// An entity whose label the request's catalog does not declare is
    /// rejected; an empty catalog keeps nothing.
    #[test]
    fn an_undeclared_label_is_rejected_by_the_catalog() {
        let catalog: LabelCatalog = [Label::new("email_address", "email")].into_iter().collect();
        let scope = Scope::new().with_catalog(catalog);
        let out = CatalogFilter.apply(
            vec![entity("email_address", 0.9), entity("iban", 0.9)],
            &Context::new(&scope),
        );

        assert_eq!(out.kept().len(), 1);
        assert_eq!(*out.kept()[0].label(), LabelRef::new("email_address"));
        let [(dropped, winner)] = out.dropped() else {
            panic!("one drop: {:?}", out.dropped());
        };
        assert_eq!(*dropped.label(), LabelRef::new("iban"));
        assert_eq!(*winner, None);

        let none = CatalogFilter.run(vec![entity("email_address", 0.9), entity("iban", 0.9)]);
        assert!(none.kept().is_empty());
        assert_eq!(none.dropped().len(), 2);
    }
}
