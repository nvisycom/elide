//! A [`Recognizer`] decorator that rescales its findings' confidence.
//!
//! [`Calibrated`] brings a recognizer's scores into line with the
//! others' before overlaps are fused and arbitrated.

mod calibration;
mod multiplier;

use elide_core::entity::Entity;
use elide_core::entity::audit::{AuditEvent, Calibrated as CalibratedEvent};
use elide_core::modality::Modality;
use elide_core::recognition::{Context, Recognizer, Subject};
use elide_core::{Component, Result};

pub use self::calibration::Calibration;
pub use self::multiplier::Multiplier;

/// Wraps a [`Recognizer`] so that each finding's confidence is scaled by its
/// [`Calibration`].
///
/// Calibration describes one recognizer's score shape, so it is attached to
/// the recognizer itself: no name to match, and a finding is calibrated as
/// it is found, before any layer fuses or arbitrates it. Each rescaled
/// finding records a calibration event with the multiplier; one whose
/// multiplier is `1.0` is returned as found.
///
/// Because it is itself a `Recognizer` over the same modality, it drops in
/// anywhere the inner recognizer goes and composes with the other decorators.
#[derive(Component)]
pub struct Calibrated<R> {
    #[component(delegate)]
    inner: R,
    calibration: Calibration,
}

impl<R> Calibrated<R> {
    /// Scale `inner`'s findings by `calibration`.
    #[must_use]
    pub fn new(inner: R, calibration: Calibration) -> Self {
        Self { inner, calibration }
    }

    /// The wrapped recognizer.
    pub fn inner(&self) -> &R {
        &self.inner
    }

    /// The calibration applied.
    pub fn calibration(&self) -> &Calibration {
        &self.calibration
    }

    /// Scale each of `entities` by its label's multiplier.
    fn calibrate<M: Modality>(&self, mut entities: Vec<Entity<M>>) -> Vec<Entity<M>>
    where
        R: Component,
    {
        for entity in &mut entities {
            let factor = self.calibration.factor(entity.label());
            if factor != Multiplier::IDENTITY {
                let scaled = entity.confidence().saturating_mul(factor.get());
                let calibrated = CalibratedEvent::new(self.inner.id(), factor.get());
                entity.record(AuditEvent::new(scaled, calibrated));
            }
        }
        entities
    }
}

#[async_trait::async_trait]
impl<M: Modality, R: Recognizer<M>> Recognizer<M> for Calibrated<R> {
    async fn recognize(&self, subject: &Subject<M>, ctx: &Context<'_>) -> Result<Vec<Entity<M>>> {
        let entities = self.inner.recognize(subject, ctx).await?;
        Ok(self.calibrate(entities))
    }

    async fn recognize_batch(
        &self,
        subjects: &[&Subject<M>],
        ctx: &Context<'_>,
    ) -> Result<Vec<Vec<Entity<M>>>> {
        let per_subject = self.inner.recognize_batch(subjects, ctx).await?;
        Ok(per_subject
            .into_iter()
            .map(|entities| self.calibrate(entities))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use elide_core::entity::audit::{AuditKind, Pattern, PatternRule};
    use elide_core::entity::{Birth, builtins};
    use elide_core::modality::text::{Text, TextData, TextLocation};
    use elide_core::primitive::{ComponentId, Confidence};
    use elide_core::recognition::Scope;

    use super::*;

    /// Finds a person and an organization name, both at full confidence.
    #[derive(Component)]
    #[component(name = "two-finds", version = "0")]
    struct TwoFinds;

    #[async_trait::async_trait]
    impl Recognizer<Text> for TwoFinds {
        async fn recognize(
            &self,
            _subject: &Subject<Text>,
            _ctx: &Context<'_>,
        ) -> Result<Vec<Entity<Text>>> {
            Ok([
                builtins::PERSON_NAME.to_ref(),
                builtins::ORGANIZATION_NAME.to_ref(),
            ]
            .into_iter()
            .map(|label| {
                let at = TextLocation::inline(0..4);
                Entity::new(Birth::new(
                    Confidence::MAX,
                    Pattern::new(
                        ComponentId::new("two-finds", "1"),
                        label,
                        at,
                        PatternRule::default(),
                    ),
                ))
            })
            .collect())
        }
    }

    /// Each finding is scaled by its label's multiplier, on the single and the
    /// batch path alike, and a multiplier of `1.0` records nothing.
    #[tokio::test]
    async fn findings_scale_by_their_label() {
        let calibration = Calibration::default().with_label(
            builtins::ORGANIZATION_NAME.to_ref(),
            Multiplier::new(0.5).unwrap(),
        );
        let recognizer = Calibrated::new(TwoFinds, calibration);
        let scope = Scope::new();
        let ctx = Context::new(&scope);
        let subject = Subject::<Text>::new(TextData::new("Jane".to_owned()));

        let single = recognizer.recognize(&subject, &ctx).await.unwrap();
        let batch = recognizer.recognize_batch(&[&subject], &ctx).await.unwrap();

        for found in [single, batch.into_iter().next().unwrap()] {
            let [person, organization] = &found[..] else {
                panic!("two findings");
            };
            assert_eq!(person.confidence(), Confidence::MAX);
            assert_eq!(person.audit().events().len(), 1);
            assert_eq!(organization.confidence(), Confidence::new(0.5).unwrap());
            assert!(matches!(
                organization.audit().events().last().unwrap().kind,
                AuditKind::Calibrated(_)
            ));
            assert!(organization.audit().verify().is_ok());
        }
    }

    /// The decorator takes the inner recognizer's identity.
    #[test]
    fn it_is_named_after_its_recognizer() {
        let recognizer =
            Calibrated::new(TwoFinds, Calibration::scale(Multiplier::new(0.5).unwrap()));
        assert_eq!(recognizer.id(), TwoFinds.id());
    }
}
