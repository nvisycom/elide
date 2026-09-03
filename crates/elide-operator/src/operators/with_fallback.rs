//! [`TryOperator`] and [`WithFallback`]: an operator that may decline a
//! value, plus a wrapper that runs another operator when it does.
//!
//! Some operators only apply to values of a certain *shape*, [`Clamp`]
//! reshapes numbers, [`GeneralizeDate`] reshapes dates. When the value isn't
//! that shape (a non-numeric age, a free-text date), the operator hasn't
//! *failed*; it simply doesn't apply, and something else should decide
//! what happens to that value.
//!
//! A raw "try the next operator when this one errors" chain can't express
//! this: it can't tell "not applicable" (a routing decision) from a *hard*
//! error, a key fetch timed out, encryption failed, that must abort the
//! redaction rather than be silently swallowed into a weaker treatment.
//! So the two are kept distinct:
//!
//! - [`TryOperator`] refines [`Operator`] with [`try_anonymize`], which
//!   returns `Ok(None)` for "not applicable", separate from `Err`.
//! - [`WithFallback`] wraps a [`TryOperator`] and any [`Operator`],
//!   including a caller's own, running the fallback exactly when the
//!   primary declines. A genuine `Err` from either side still propagates.
//!
//! [`try_anonymize`]: TryOperator::try_anonymize
//! [`Clamp`]: super::Clamp
//! [`GeneralizeDate`]: super::GeneralizeDate

use elide_core::Result;
use elide_core::entity::Entity;
use elide_core::modality::Modality;
use elide_core::redaction::{LeakProfile, Operator, OperatorId};

/// An [`Operator`] that may decline to apply to a given value.
///
/// The refinement for operators like [`Clamp`] and [`GeneralizeDate`] that
/// only reshape values of a particular shape. [`try_anonymize`] returns
/// `Ok(Some(replacement))` when it applied, `Ok(None)` when the value
/// wasn't its shape, a distinct, first-class outcome, and `Err` only for
/// a genuine failure.
///
/// It is a supertrait of [`Operator`]: a `TryOperator` *is* an operator.
/// Its [`Operator::anonymize`] is the "used on its own" behaviour (these
/// operators erase a declined value, the safe default); wrap it in
/// [`WithFallback`] to choose a different treatment for declined values.
///
/// [`try_anonymize`]: TryOperator::try_anonymize
/// [`Clamp`]: super::Clamp
/// [`GeneralizeDate`]: super::GeneralizeDate
#[async_trait::async_trait]
pub trait TryOperator<M: Modality>: Operator<M> {
    /// Compute the replacement for `entity`, or `Ok(None)` if this operator
    /// doesn't apply to `data`.
    ///
    /// `Ok(None)` means "not my shape", defer to a fallback. It is never
    /// used for a real failure; that is an `Err`, which aborts the batch.
    async fn try_anonymize(
        &self,
        entity: &Entity<M>,
        data: &M::Data,
    ) -> Result<Option<M::Replacement>>;
}

/// Run a [`TryOperator`], falling back to another [`Operator`] when it
/// declines the value.
///
/// Runs `primary` first. When it produces a replacement, that is the
/// result. When it *declines* (`try_anonymize` returns `Ok(None)`, the
/// value wasn't the primary's shape), `fallback` runs instead. The
/// fallback is any [`Operator`], so a caller composes their own treatment
/// for the leftover values:
///
/// ```
/// # use elide_operator::operators::{Clamp, Erase, WithFallback};
/// // Ages: cap at 90, and erase anything that isn't a number.
/// let op = WithFallback::new(Clamp::new().with_ceiling(90.0, "90 or older"), Erase);
/// # let _ = op;
/// ```
///
/// A hard error from either operator still propagates, only the primary's
/// "not applicable" is caught here.
#[derive(Debug, Clone)]
pub struct WithFallback<P, F> {
    primary: P,
    fallback: F,
}

impl<P, F> WithFallback<P, F> {
    /// Run `primary`, deferring to `fallback` when it declines a value.
    pub fn new(primary: P, fallback: F) -> Self {
        Self { primary, fallback }
    }
}

#[async_trait::async_trait]
impl<M, P, F> Operator<M> for WithFallback<P, F>
where
    M: Modality,
    P: TryOperator<M>,
    F: Operator<M>,
{
    fn id(&self) -> OperatorId {
        // One composite identity naming both paths a value might take, e.g.
        // `generalize_date@1.0.0-or-else-erase@1.0.0`. The audit records this
        // whichever branch fired, so it never falsely claims a value was
        // erased when it was reshaped, nor the reverse.
        composite_id(&self.primary.id(), &self.fallback.id())
    }

    fn leak_profile(&self) -> LeakProfile {
        // A value could take either path, so the profile a caller can
        // *rely on* is the leakier (smaller) of the two.
        self.primary
            .leak_profile()
            .min(self.fallback.leak_profile())
    }

    async fn anonymize(&self, entity: &Entity<M>, data: &M::Data) -> Result<M::Replacement> {
        match self.primary.try_anonymize(entity, data).await? {
            Some(replacement) => Ok(replacement),
            None => self.fallback.anonymize(entity, data).await,
        }
    }
}

/// Build the composite operator id for a [`WithFallback`], naming both the
/// primary and the fallback with their versions (e.g.
/// `generalize_date@1.0.0-or-else-erase@1.0.0`). Folding in each operand's
/// version keeps the audit trail exact: it records which build of *both*
/// paths a value could have taken, not just their names.
fn composite_id(primary: &OperatorId, fallback: &OperatorId) -> OperatorId {
    OperatorId::new(format!("{primary}-or-else-{fallback}"), "1.0.0")
}

#[cfg(test)]
mod tests {
    use elide_core::entity::LabelRef;
    use elide_core::entity::audit::{AuditEvent, AuditLog, PatternEvent};
    use elide_core::modality::text::{Text, TextData, TextLocation, TextReplacement};
    use elide_core::primitive::Confidence;

    use super::*;
    use crate::operators::{Clamp, Erase, Keep};

    fn entity() -> Entity<Text> {
        let location = TextLocation::new(0, 3);
        let event = AuditEvent::pattern(
            "t",
            Confidence::MAX,
            location.clone(),
            PatternEvent::default(),
        );
        Entity::new(LabelRef::new("age"), location, AuditLog::new(event))
    }

    #[tokio::test]
    async fn primary_result_is_used_when_it_applies() {
        let op = WithFallback::new(Clamp::new().with_ceiling(90.0, "90 or older"), Erase);
        let out = Operator::<Text>::anonymize(&op, &entity(), &TextData::new("94"))
            .await
            .unwrap();
        assert_eq!(out, TextReplacement::substituted("90 or older"));
    }

    #[tokio::test]
    async fn fallback_runs_when_the_primary_declines() {
        // A non-number: Clamp declines, so the custom fallback (Keep) runs
        // instead of the safe default (erase).
        let op = WithFallback::new(Clamp::new().with_ceiling(90.0, "x"), Keep);
        let out = Operator::<Text>::anonymize(&op, &entity(), &TextData::new("N/A"))
            .await
            .unwrap();
        assert_eq!(out, TextReplacement::substituted("N/A"));
    }

    #[tokio::test]
    async fn composite_id_names_both_paths_with_versions() {
        let op = WithFallback::new(Clamp::new().with_ceiling(90.0, "x"), Erase);
        // Each operand contributes name@version, so the audit records which
        // build of both paths a value could have taken.
        assert_eq!(
            Operator::<Text>::id(&op).name,
            "clamp@1.0.0-or-else-erase@1.0.0"
        );
    }

    #[tokio::test]
    async fn leak_profile_is_the_leakier_of_the_two() {
        // Clamp is Partial, Keep is Recoverable (leakier). The pair reports
        // Recoverable, the guarantee a caller can rely on.
        let op = WithFallback::new(Clamp::new().with_ceiling(90.0, "x"), Keep);
        assert_eq!(
            Operator::<Text>::leak_profile(&op),
            LeakProfile::Recoverable
        );
    }
}
