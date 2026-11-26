//! [`ReviewPolicy`]: which reviewer edits an orchestrator accepts, the
//! [`ProposedEdit`] it judges, and the standard policies, [`Protect`] and
//! [`RequireReason`], combined with [`and`](ReviewPolicy::and).

use std::collections::HashSet;

use elide_core::entity::LabelRef;
use elide_core::entity::audit::{Attribution, ManualIntent};
use elide_core::{Error, Result};
use uuid::Uuid;

use crate::PartId;

/// Which reviewer edits [`Orchestrator::review`] accepts.
///
/// The orchestrator shows the policy each edit of a review in turn, as a
/// [`ProposedEdit`], against the part as the edits before it left it. An `Err`
/// refuses the edit, and with it the whole review: the report is left as it
/// was. Return one of kind [`Refused`](elide_core::ErrorKind::Refused).
///
/// Each standard policy checks one thing: [`Protect`] keeps labels from being
/// narrowed, and [`RequireReason`] wants every narrowing edit explained.
/// Combine them, and policies of the deployment's own, with
/// [`and`](Self::and); a closure taking a `&ProposedEdit` is a policy too,
/// such as one letting only some actors narrow.
///
/// The actor a policy reads is the one the deployment named when it applied
/// the review, from however it authenticated the reviewer, never a name the
/// review carries.
///
/// [`Orchestrator::review`]: crate::Orchestrator::review
pub trait ReviewPolicy: Send + Sync {
    /// Accept `edit`, or refuse it.
    ///
    /// # Errors
    ///
    /// Any error refuses the edit and the review it belongs to.
    fn check(&self, edit: &ProposedEdit<'_>) -> Result<()>;

    /// A policy that accepts an edit only if both this one and `other` do.
    #[must_use]
    fn and<P: ReviewPolicy>(self, other: P) -> And<Self, P>
    where
        Self: Sized,
    {
        And(self, other)
    }
}

impl<F> ReviewPolicy for F
where
    F: Fn(&ProposedEdit<'_>) -> Result<()> + Send + Sync,
{
    fn check(&self, edit: &ProposedEdit<'_>) -> Result<()> {
        self(edit)
    }
}

/// Two policies, both of which must accept an edit, from
/// [`ReviewPolicy::and`]. The first to refuse decides the error.
#[derive(Debug, Clone)]
pub struct And<A, B>(A, B);

impl<A: ReviewPolicy, B: ReviewPolicy> ReviewPolicy for And<A, B> {
    fn check(&self, edit: &ProposedEdit<'_>) -> Result<()> {
        self.0.check(edit)?;
        self.1.check(edit)
    }
}

/// One reviewer edit as a [`ReviewPolicy`] judges it, whatever its modality.
#[derive(Debug, Clone, Copy)]
pub struct ProposedEdit<'a> {
    /// The part the edit changes.
    pub part: &'a PartId,
    /// The decision the edit records, with the label it sets: an include's or
    /// a relabel's.
    pub decision: &'a ManualIntent,
    /// The entities whose redaction the edit narrows: each it touches, and
    /// each in an overlap cluster with one it touches, as entities that
    /// overlap, directly or through others, share one redaction under the
    /// safest operator among them. Any already suppressed is left out. Empty
    /// for an include, which only adds to what is hidden.
    pub narrowed: &'a [Narrowed],
    /// Who made the edit, as the deployment named them when it applied the
    /// review.
    pub by: &'a str,
    /// Why the review says it was made.
    pub because: Option<&'a Attribution>,
}

impl ProposedEdit<'_> {
    /// Whether the edit could leave content unredacted that would have been
    /// hidden: a suppression, a relabel, or a relocation of an entity not
    /// already suppressed.
    #[must_use]
    pub fn narrows(&self) -> bool {
        !self.narrowed.is_empty()
    }
}

/// An entity whose redaction a [`ProposedEdit`] narrows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Narrowed {
    /// The entity's id.
    pub id: Uuid,
    /// Its label before the edit.
    pub label: LabelRef,
}

/// Refuses every edit that narrows the redaction of an entity of a protected
/// label: no suppression, relabel, or relocation of one, or of an entity in
/// an overlap cluster with one. A region suppression containing one is
/// refused too.
#[derive(Debug, Clone, Default)]
pub struct Protect(HashSet<LabelRef>);

impl Protect {
    /// Protect each of `labels`.
    #[must_use]
    pub fn labels(labels: impl IntoIterator<Item = impl Into<LabelRef>>) -> Self {
        Self(labels.into_iter().map(Into::into).collect())
    }
}

impl ReviewPolicy for Protect {
    fn check(&self, edit: &ProposedEdit<'_>) -> Result<()> {
        match edit
            .narrowed
            .iter()
            .find(|entity| self.0.contains(&entity.label))
        {
            Some(entity) => Err(Error::refused(format!(
                "an edit of part `{}` narrows the redaction of entity `{}`, whose label `{}` is protected",
                edit.part,
                entity.id,
                entity.label.as_str()
            ))),
            None => Ok(()),
        }
    }
}

/// Refuses every narrowing edit that does not say why it was made.
#[derive(Debug, Clone, Copy, Default)]
pub struct RequireReason;

impl ReviewPolicy for RequireReason {
    fn check(&self, edit: &ProposedEdit<'_>) -> Result<()> {
        if edit.narrows() && edit.because.is_none() {
            return Err(Error::refused(format!(
                "an edit of part `{}` narrows its redaction without a reason",
                edit.part
            )));
        }
        Ok(())
    }
}
