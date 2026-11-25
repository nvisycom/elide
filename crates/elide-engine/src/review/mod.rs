//! [`Review`]: a reviewer's edits to a [`Report`], made against one version
//! of it, and the [`ReviewPolicy`] that decides which are accepted.
//!
//! A review is the only way to change what a report says. It names the report
//! it was made against by digest, carries edits rather than an edited report,
//! and is applied by [`Orchestrator::review`], which checks each edit against
//! the policy and records it on the entities it touches as a `Manual` audit
//! event. So a review layer that never holds the report's authority, a
//! browser, a review queue, can still say what it wants changed, and the
//! trusted side decides what it accepts.
//!
//! [`Orchestrator::review`]: crate::Orchestrator::review

mod edit;
mod group;
mod policy;

use std::collections::HashSet;
use std::fmt;

use elide_core::entity::LabelRef;
use elide_core::entity::audit::{Attribution, AuditHash};
use elide_core::modality::Modality;
use elide_core::{Error, Result};
use serde::Serialize;
use serde::de::DeserializeOwned;
use uuid::Uuid;

pub use self::edit::{Change, Edit};
pub(crate) use self::group::EditGroup;
pub use self::policy::{
    And, Narrowed, ProposedEdit, Protect, RequireActor, RequireReason, ReviewPolicy,
};
use crate::wire::part_fields;
use crate::{PartId, Report};

/// A reviewer's edits to a [`Report`], made against one version of it.
///
/// Start one from the report a reviewer is shown with [`new`](Self::new), add
/// edits part by part through [`part`](Self::part), and apply it with
/// [`Orchestrator::review`]. It serializes, so it can travel from a review
/// layer to the side that holds the report; read it back against its
/// [`Seal`](crate::Seal) with [`Restorer::restore`].
///
/// A review is bound to the [digest](Report::digest) of the report it was made
/// against, and applies only to that report: one made against another
/// report, or an earlier version of this one (applying a review changes the
/// digest), is refused. A review applied once cannot be applied again.
///
/// What a review cannot protect itself against is its carrier: an edit
/// dropped on the way leaves the entity as the analysis found it, and an
/// [include](ReviewPart::include) dropped leaves content unredacted. Carry a
/// review over a channel the deployment authenticates, or seal it on a
/// reviewer's side that holds the [`SealKey`](crate::SealKey), with
/// [`Restorer::seal`](crate::Restorer::seal) under
/// [`with_seal_key`](crate::Restorer::with_seal_key).
///
/// [`Orchestrator::review`]: crate::Orchestrator::review
/// [`Restorer::restore`]: crate::Restorer::restore
pub struct Review {
    /// The digest of the report the review was made against.
    pub(crate) against: AuditHash,
    /// Each part's edits, in the order the part was first edited.
    pub(crate) parts: Vec<(PartId, Box<dyn EditGroup>)>,
}

impl Review {
    /// An empty review of `report`, bound to its current digest.
    #[must_use]
    pub fn new(report: &Report) -> Self {
        Self {
            against: report.digest(),
            parts: Vec::new(),
        }
    }

    /// The digest of the report the review was made against.
    #[must_use]
    pub fn against(&self) -> AuditHash {
        self.against
    }

    /// Whether the review holds no edits.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.parts.iter().all(|(_, edits)| edits.is_empty())
    }

    /// The part `id`, as modality `M`, to add edits to.
    ///
    /// # Errors
    ///
    /// [`MalformedInput`](elide_core::ErrorKind::MalformedInput) when the
    /// review already edits part `id` as another modality.
    pub fn part<M: Modality>(&mut self, id: &PartId) -> Result<ReviewPart<'_, M>>
    where
        M::Location: Serialize + DeserializeOwned,
    {
        let index = match self.parts.iter().position(|(part, _)| part == id) {
            Some(index) => index,
            None => {
                self.parts
                    .push((id.clone(), Box::new(Vec::<Edit<M>>::new())));
                self.parts.len() - 1
            }
        };
        let edits = self.parts[index]
            .1
            .as_any_mut()
            .downcast_mut::<Vec<Edit<M>>>()
            .ok_or_else(|| {
                Error::malformed(format!(
                    "the review already edits part `{id}` as another modality than {}",
                    M::NAME
                ))
            })?;
        Ok(ReviewPart {
            edits,
            by: None,
            because: None,
        })
    }

    /// The edits of part `id`, as modality `M`, or `None` when the review
    /// holds none of that modality there.
    #[must_use]
    pub fn edits<M: Modality>(&self, id: &PartId) -> Option<&[Edit<M>]> {
        self.parts
            .iter()
            .find(|(part, _)| part == id)
            .and_then(|(_, edits)| edits.as_any().downcast_ref::<Vec<Edit<M>>>())
            .map(Vec::as_slice)
    }

    /// Apply the review to `report` under `policy`: every edit or none.
    ///
    /// The review must have been made against `report` as it is now, and must
    /// change it: one that changes nothing would leave the digest as it was,
    /// and so could be applied again. Each part's edits are checked and
    /// applied in turn on a copy, and the copies replace the parts only once
    /// every edit passed, so a refused edit leaves the report as it was.
    pub(crate) fn apply(&self, report: &mut Report, policy: &dyn ReviewPolicy) -> Result<()> {
        if !self.against.matches(&report.digest()) {
            return Err(Error::integrity(
                "the review was made against another report, or an earlier version of this one",
            ));
        }
        // An entity id names one entity across the whole report, so an include
        // may not reuse one any part holds, or another include gave.
        let mut ids: HashSet<Uuid> = report
            .parts
            .values()
            .flat_map(|entities| entities.ids())
            .collect();
        for (part, edits) in &self.parts {
            if let Some(id) = edits.included().into_iter().find(|id| !ids.insert(*id)) {
                return Err(Error::malformed(format!(
                    "an include in part `{part}` names entity `{id}`, which the report already holds"
                )));
            }
        }
        let mut reviewed = Vec::with_capacity(self.parts.len());
        let mut changed = false;
        // A group with no edits is one the seal leaves out, and so is skipped
        // before its modality is matched against the part's.
        for (id, edits) in self.parts.iter().filter(|(_, edits)| !edits.is_empty()) {
            // A part with no entities is one the seal leaves out, so it is
            // edited as if absent, whatever modality it was held as.
            let empty;
            let entities = match report.parts.get(id) {
                Some(entities) if !entities.is_empty() => entities.as_ref(),
                _ => {
                    empty = edits.empty_entities();
                    empty.as_ref()
                }
            };
            let (entities, edited) = edits.apply(id, entities, policy)?;
            changed |= edited;
            reviewed.push((id.clone(), entities));
        }
        // A review that changes nothing would leave the report's digest as it
        // was, so it could be applied again and again; refuse it instead.
        if !changed {
            return Err(Error::malformed("the review changes nothing in the report"));
        }
        report.parts.extend(reviewed);
        Ok(())
    }

    /// The fields the review's seal covers: the report's digest, then each
    /// part by path with its modality and its edits in order. Parts are
    /// ordered by path, as they apply independently.
    pub(crate) fn seal_fields(&self) -> Vec<Vec<u8>> {
        // An empty group is left out, as a restorer drops one whose modality
        // it does not know.
        let parts = self
            .parts
            .iter()
            .filter(|(_, edits)| !edits.is_empty())
            .map(|(id, edits)| {
                let mut body = Vec::new();
                edits.fields(&mut body);
                (id, edits.modality_name(), body)
            });
        let mut fields = vec![self.against.as_bytes().to_vec()];
        fields.extend(part_fields(parts));
        fields
    }
}

impl fmt::Debug for Review {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let parts: Vec<_> = self
            .parts
            .iter()
            .map(|(id, edits)| (id, edits.modality_name()))
            .collect();
        f.debug_struct("Review")
            .field("against", &self.against)
            .field("parts", &parts)
            .finish_non_exhaustive()
    }
}

/// One part of a [`Review`], typed to its modality, to add edits to.
///
/// Returned by [`Review::part`]. Each edit is attributed to
/// [`by`](Self::by) and [`because`](Self::because) when set, which the
/// [`ReviewPolicy`] may require, and which the applied report records on the
/// entities the edit touches.
#[derive(Debug)]
pub struct ReviewPart<'r, M: Modality> {
    edits: &'r mut Vec<Edit<M>>,
    by: Option<String>,
    because: Option<Attribution>,
}

impl<M: Modality> ReviewPart<'_, M> {
    /// Attribute the edits added through this view to `actor`.
    #[must_use]
    pub fn by(mut self, actor: impl Into<String>) -> Self {
        self.by = Some(actor.into());
        self
    }

    /// Record `attribution` as the reason for the edits added through this
    /// view.
    #[must_use]
    pub fn because(mut self, attribution: impl Into<Attribution>) -> Self {
        self.because = Some(attribution.into());
        self
    }

    /// Add an entity of `label` at `location`, one detection missed, and
    /// return the id it will have, which later edits can address.
    ///
    /// A reviewer's assertion is certain, so the entity carries full
    /// confidence. The location can be a span, or a mark naming text by where
    /// it sits, which the apply resolves to the span it covers.
    pub fn include(&mut self, label: impl Into<LabelRef>, location: M::Location) -> Uuid {
        let entity = Uuid::now_v7();
        self.push(Change::Include {
            entity,
            label: label.into(),
            location,
        });
        entity
    }

    /// Leave the entity `entity` unredacted.
    pub fn suppress(&mut self, entity: Uuid) -> &mut Self {
        self.push(Change::Suppress { entity })
    }

    /// Leave every entity `region` contains unredacted, as the review finds
    /// the part when it is applied. An entity reaching outside `region` is
    /// kept, so nothing outside it is spared.
    pub fn suppress_within(&mut self, region: M::Location) -> &mut Self {
        self.push(Change::SuppressWithin { region })
    }

    /// Give the entity `entity` the label `label`.
    pub fn relabel(&mut self, entity: Uuid, label: impl Into<LabelRef>) -> &mut Self {
        self.push(Change::Relabel {
            entity,
            label: label.into(),
        })
    }

    /// Move the entity `entity` to `location`.
    pub fn relocate(&mut self, entity: Uuid, location: M::Location) -> &mut Self {
        self.push(Change::Relocate { entity, location })
    }

    /// Add `change`, attributed as this view says.
    fn push(&mut self, change: Change<M>) -> &mut Self {
        self.edits.push(Edit {
            change,
            by: self.by.clone(),
            because: self.because.clone(),
        });
        self
    }
}

#[cfg(test)]
mod tests {
    use elide_core::ErrorKind;
    use elide_core::entity::Entity;
    use elide_core::entity::audit::{AuditKind, Manual, ManualIntent};
    use elide_core::modality::metadata::Metadata;
    use elide_core::modality::text::{Text, TextLocation};
    use elide_core::primitive::Confidence;

    use super::*;
    use crate::analysis::test_support::{doc, text_entity, text_entity_at as at};
    use crate::{Restorer, Seal, SealKey};

    /// The `Manual` decisions on `entity`'s trail, with who made them.
    fn decisions(entity: &Entity<Text>) -> Vec<(String, ManualIntent, TextLocation)> {
        entity
            .audit()
            .events()
            .iter()
            .filter_map(|event| match &event.kind {
                AuditKind::Manual(manual) => Some((
                    event.source.to_string(),
                    manual.intent.clone(),
                    manual.location.clone(),
                )),
                _ => None,
            })
            .collect()
    }

    fn apply(report: &mut Report, review: &Review) -> Result<()> {
        review.apply(report, &|_: &ProposedEdit<'_>| Ok(()))
    }

    #[test]
    fn an_included_entity_is_a_reviewers_assertion() {
        let mut report = Report::new();
        let mut review = Review::new(&report);
        let id = review
            .part::<Text>(&doc())
            .unwrap()
            .by("reviewer-7")
            .because(Attribution::freeform("missed by detection"))
            .include("PERSON_NAME", TextLocation::inline(4..12));
        apply(&mut report, &review).unwrap();

        let part = report.part::<Text>(&doc()).unwrap();
        let [entity] = part.entities() else {
            panic!("one entity");
        };
        assert_eq!(entity.id(), id, "the entity has the id the review gave it");
        assert_eq!(*entity.label(), LabelRef::new("PERSON_NAME"));
        assert_eq!(entity.confidence(), Confidence::MAX);
        assert_eq!(
            decisions(entity),
            [(
                "reviewer-7".to_owned(),
                ManualIntent::Include {
                    label: LabelRef::new("PERSON_NAME")
                },
                TextLocation::inline(4..12),
            )]
        );
        assert!(entity.audit().verify().is_ok());
        assert_eq!(part.unsuppressed().count(), 1);
    }

    /// A later edit in the same review can address an entity the review
    /// included, and an include cannot reuse an id the part holds.
    #[test]
    fn a_review_can_amend_what_it_included() {
        let detected = text_entity("EMAIL_ADDRESS");
        let taken = detected.id();
        let mut report = Report::new().insert_part::<Text>(doc(), vec![detected]);
        let mut review = Review::new(&report);
        let mut part = review.part::<Text>(&doc()).unwrap();
        let id = part.include("PERSON_NAME", TextLocation::inline(4..12));
        part.relabel(id, "ORGANIZATION");
        apply(&mut report, &review).unwrap();
        let included = report.entity_anywhere::<Text>(id).unwrap();
        assert_eq!(*included.label(), LabelRef::new("ORGANIZATION"));

        // An include may not reuse an id the report holds, in its own part or
        // any other.
        for part in [doc(), doc().child("word/media/image1.png")] {
            let mut review = Review::new(&report);
            review.parts.push((
                part,
                Box::new(vec![Edit::<Text> {
                    change: Change::Include {
                        entity: taken,
                        label: LabelRef::new("X"),
                        location: TextLocation::inline(0..1),
                    },
                    by: None,
                    because: None,
                }]),
            ));
            let err = apply(&mut report, &review).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::MalformedInput);
        }
    }

    #[test]
    fn a_relabel_and_a_relocation_record_what_they_set() {
        let entity = text_entity("EMAIL_ADDRESS");
        let id = entity.id();
        let mut report = Report::new().insert_part::<Text>(doc(), vec![entity]);
        let mut review = Review::new(&report);
        review
            .part::<Text>(&doc())
            .unwrap()
            .by("reviewer-7")
            .relabel(id, "PHONE_NUMBER")
            .relocate(id, TextLocation::inline(1..3));
        apply(&mut report, &review).unwrap();

        let entity = report.entity_anywhere::<Text>(id).unwrap();
        assert_eq!(*entity.label(), LabelRef::new("PHONE_NUMBER"));
        assert_eq!(*entity.location(), TextLocation::inline(1..3));
        assert_eq!(
            decisions(entity),
            [
                (
                    "reviewer-7".to_owned(),
                    ManualIntent::Relabel {
                        label: LabelRef::new("PHONE_NUMBER")
                    },
                    TextLocation::inline(0..4),
                ),
                (
                    "reviewer-7".to_owned(),
                    ManualIntent::Relocate,
                    TextLocation::inline(1..3),
                ),
            ]
        );
        assert!(!entity.is_suppressed());
        assert!(entity.audit().verify().is_ok());
    }

    #[test]
    fn a_region_suppresses_only_what_it_contains() {
        let inside = at("EMAIL_ADDRESS", 4..10);
        let crossing = at("PERSON_NAME", 15..25);
        let outside = at("PHONE_NUMBER", 30..40);
        let ids = [inside.id(), crossing.id(), outside.id()];
        let mut report = Report::new().insert_part::<Text>(doc(), vec![inside, crossing, outside]);
        let region = TextLocation::inline(0..20);
        let mut review = Review::new(&report);
        review
            .part::<Text>(&doc())
            .unwrap()
            .suppress_within(region.clone());
        apply(&mut report, &review).unwrap();

        let part = report.part::<Text>(&doc()).unwrap();
        let suppressed = ids.map(|id| part.entity(id).unwrap().is_suppressed());
        assert_eq!(suppressed, [true, false, false]);
        let recorded = decisions(part.entity(ids[0]).unwrap());
        assert_eq!(recorded[0].2, region, "the decision records its region");
    }

    /// A review applies only to the report it was made against: once applied,
    /// the report's digest moves on, so neither it nor another review of the
    /// same version applies again.
    #[test]
    fn a_review_applies_once_to_the_version_it_was_made_against() {
        let entity = text_entity("EMAIL_ADDRESS");
        let id = entity.id();
        let mut report = Report::new().insert_part::<Text>(doc(), vec![entity]);
        let mut first = Review::new(&report);
        first.part::<Text>(&doc()).unwrap().suppress(id);
        let mut stale = Review::new(&report);
        stale.part::<Text>(&doc()).unwrap().relabel(id, "X");

        apply(&mut report, &first).unwrap();
        for review in [&first, &stale] {
            let err = apply(&mut report, review).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::Integrity);
        }
    }

    /// An edit naming no entity of its part fails the whole review, and the
    /// edits before it are not applied.
    #[test]
    fn a_failing_edit_leaves_the_report_as_it_was() {
        let entity = text_entity("EMAIL_ADDRESS");
        let id = entity.id();
        let mut report = Report::new().insert_part::<Text>(doc(), vec![entity]);
        let before = report.digest();
        let mut review = Review::new(&report);
        review
            .part::<Text>(&doc())
            .unwrap()
            .suppress(id)
            .suppress(Uuid::now_v7());

        let err = apply(&mut report, &review).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
        assert_eq!(report.digest(), before);
    }

    /// A protected label refuses every edit that narrows it, a relocation
    /// into a region containing the old one included, and only those.
    #[test]
    fn a_protected_label_refuses_what_narrows_it() {
        let ssn = at("SSN", 4..10);
        let id = ssn.id();
        let policy = Protect::labels(["SSN"]);
        let outcome = |edit: &dyn Fn(&mut ReviewPart<'_, Text>)| {
            let mut report = Report::new().insert_part::<Text>(doc(), vec![ssn.clone()]);
            let mut review = Review::new(&report);
            edit(&mut review.part::<Text>(&doc()).unwrap());
            review.apply(&mut report, &policy).map_err(|err| err.kind())
        };

        let refused = Err(ErrorKind::Refused);
        assert_eq!(
            outcome(&|part| {
                part.suppress(id);
            }),
            refused
        );
        assert_eq!(
            outcome(&|part| {
                part.relabel(id, "X");
            }),
            refused
        );
        assert_eq!(
            outcome(&|part| {
                part.relocate(id, TextLocation::inline(6..12));
            }),
            refused,
            "a partly overlapping relocation narrows",
        );
        assert_eq!(
            outcome(&|part| {
                part.suppress_within(TextLocation::inline(0..20));
            }),
            refused,
        );
        assert_eq!(
            outcome(&|part| {
                part.relocate(id, TextLocation::inline(0..20));
            }),
            refused,
            "a relocation narrows even into a region containing the old one",
        );
        assert_eq!(
            outcome(&|part| {
                part.include("SSN", TextLocation::inline(30..40));
            }),
            Ok(()),
        );
    }

    /// Each edit is checked as the edits before it left the part: an entity
    /// moved into a region is protected there.
    #[test]
    fn an_edit_is_checked_after_the_edits_before_it() {
        let ssn = at("SSN", 30..36);
        let id = ssn.id();
        let mut report = Report::new().insert_part::<Text>(doc(), vec![ssn]);
        let mut review = Review::new(&report);
        review
            .part::<Text>(&doc())
            .unwrap()
            .relocate(id, TextLocation::inline(0..40))
            .relocate(id, TextLocation::inline(4..10))
            .suppress_within(TextLocation::inline(0..20));

        // Relocations pass; only a suppression narrowing an SSN is refused.
        // The SSN starts at 30..36, outside 0..20, so the suppression is
        // refused only if it resolves against the relocated entity.
        let protect = Protect::labels(["SSN"]);
        let policy = |edit: &ProposedEdit<'_>| match edit.decision {
            ManualIntent::Relocate => Ok(()),
            _ => protect.check(edit),
        };
        let err = review.apply(&mut report, &policy).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Refused);

        let mut unmoved = Review::new(&report);
        unmoved
            .part::<Text>(&doc())
            .unwrap()
            .suppress_within(TextLocation::inline(0..20));
        let err = unmoved.apply(&mut report, &policy).unwrap_err();
        assert_eq!(
            err.kind(),
            ErrorKind::MalformedInput,
            "unmoved, the SSN lies outside the region, so nothing is suppressed",
        );
    }

    /// A group with no edits is skipped, so it cannot clash with the part's
    /// modality.
    #[test]
    fn an_empty_group_is_skipped() {
        let a = PartId::leaf("a.txt");
        let b = PartId::leaf("b.txt");
        let mut report = Report::new()
            .insert_part::<Text>(a.clone(), vec![at("EMAIL_ADDRESS", 0..10)])
            .insert_part::<Text>(b.clone(), vec![at("EMAIL_ADDRESS", 0..10)]);
        let mut review = Review::new(&report);
        review.part::<Metadata>(&b).unwrap();
        review
            .part::<Text>(&a)
            .unwrap()
            .include("SSN", TextLocation::inline(20..30));

        apply(&mut report, &review).unwrap();
        assert_eq!(report.part::<Text>(&a).unwrap().entities().len(), 2);
    }

    #[test]
    fn a_policy_can_require_an_actor_and_a_reason_to_narrow() {
        let entity = text_entity("EMAIL_ADDRESS");
        let id = entity.id();
        let policy = RequireActor.and(RequireReason);
        let outcome = |by: Option<&str>, because: Option<&str>| {
            let mut report = Report::new().insert_part::<Text>(doc(), vec![entity.clone()]);
            let mut review = Review::new(&report);
            let mut part = review.part::<Text>(&doc()).unwrap();
            if let Some(by) = by {
                part = part.by(by);
            }
            if let Some(because) = because {
                part = part.because(Attribution::freeform(because));
            }
            part.suppress(id);
            review.apply(&mut report, &policy).map_err(|err| err.kind())
        };

        assert_eq!(outcome(None, Some("public")), Err(ErrorKind::Refused));
        assert_eq!(outcome(Some("reviewer-7"), None), Err(ErrorKind::Refused));
        assert_eq!(outcome(Some("reviewer-7"), Some("public")), Ok(()));
    }

    /// A review that changes nothing is refused: it would leave the digest as
    /// it was, and so could be applied again and again.
    #[test]
    fn a_review_that_changes_nothing_is_refused() {
        let entity = at("EMAIL_ADDRESS", 30..40);
        let mut report = Report::new().insert_part::<Text>(doc(), vec![entity]);
        let mut review = Review::new(&report);
        review
            .part::<Text>(&doc())
            .unwrap()
            .suppress_within(TextLocation::inline(0..10));

        let before = report.digest();
        let err = apply(&mut report, &review).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::MalformedInput);
        assert_eq!(report.digest(), before);
    }

    /// An entity already suppressed is not redacted, so no edit narrows it,
    /// even under a policy that protects its label.
    #[test]
    fn an_already_suppressed_entity_is_not_narrowed() {
        let mut ssn = at("SSN", 4..10);
        let id = ssn.id();
        let location = ssn.location().clone();
        ssn.record_manual(Manual::new(ManualIntent::Suppress, location), None);
        let mut report = Report::new().insert_part::<Text>(doc(), vec![ssn]);
        let mut review = Review::new(&report);
        review
            .part::<Text>(&doc())
            .unwrap()
            .relabel(id, "PUBLIC_ID");

        review
            .apply(&mut report, &Protect::labels(["SSN"]))
            .unwrap();
        assert_eq!(
            *report.entity_anywhere::<Text>(id).unwrap().label(),
            LabelRef::new("PUBLIC_ID")
        );
    }

    /// A part with no entities is edited as if absent, whatever modality it
    /// was held as, as the seal leaves it out.
    #[test]
    fn an_empty_part_is_edited_as_absent() {
        let mut report = Report::new().insert_part::<Metadata>(doc(), Vec::new());
        let mut review = Review::new(&report);
        let id = review
            .part::<Text>(&doc())
            .unwrap()
            .include("PERSON_NAME", TextLocation::inline(0..4));

        apply(&mut report, &review).unwrap();
        assert!(report.entity_anywhere::<Text>(id).is_some());
    }

    /// A closure is a policy: here, only a lead may narrow, and it sees which
    /// entities an edit narrows.
    #[test]
    fn a_closure_is_a_policy() {
        let entity = text_entity("EMAIL_ADDRESS");
        let id = entity.id();
        let only_leads_narrow = |edit: &ProposedEdit<'_>| {
            if edit.narrows() && edit.by != Some("lead") {
                return Err(Error::refused("only a lead may narrow"));
            }
            assert!(edit.narrowed.iter().all(|narrowed| narrowed.id == id));
            Ok(())
        };
        let outcome = |by: &str| {
            let mut report = Report::new().insert_part::<Text>(doc(), vec![entity.clone()]);
            let mut review = Review::new(&report);
            let mut part = review.part::<Text>(&doc()).unwrap().by(by);
            part.include("PERSON_NAME", TextLocation::inline(8..12));
            part.suppress(id);
            review
                .apply(&mut report, &only_leads_narrow)
                .map_err(|err| err.kind())
        };

        assert_eq!(outcome("reviewer-7"), Err(ErrorKind::Refused));
        assert_eq!(outcome("lead"), Ok(()));
    }

    /// A review reads back against the seal it was written with; one with an
    /// edit dropped on the way does not.
    #[test]
    fn a_review_reads_back_only_against_its_seal() {
        let entity = text_entity("EMAIL_ADDRESS");
        let id = entity.id();
        let report = Report::new().insert_part::<Text>(doc(), vec![entity]);
        let mut review = Review::new(&report);
        let mut part = review.part::<Text>(&doc()).unwrap().by("reviewer-7");
        part.suppress(id);
        part.include("PERSON_NAME", TextLocation::inline(8..12));
        let key = SealKey::new([7; 32]);
        let unkeyed = Restorer::new().with_modality::<Text>();
        let keyed = Restorer::new()
            .with_modality::<Text>()
            .with_seal_key(key.clone());
        let cases = [
            (&unkeyed, unkeyed.seal(&review)),
            (&keyed, keyed.seal(&review)),
        ];
        let json = serde_json::to_value(&review).unwrap();

        for (restorer, seal) in &cases {
            let read: Review = restorer
                .restore(seal, json.clone())
                .expect("reads back intact");
            assert_eq!(restorer.seal(&read), restorer.seal(&review));
            assert_eq!(read.edits::<Text>(&doc()).unwrap().len(), 2);
        }

        let mut dropped = json.clone();
        dropped["parts"][0]["edits"]
            .as_array_mut()
            .unwrap()
            .remove(1);
        for (restorer, seal) in &cases {
            let err = restorer
                .restore::<Review, _>(seal, dropped.clone())
                .unwrap_err();
            assert_eq!(err.kind(), ErrorKind::Integrity);
        }

        // The reader chooses the scheme: a keyed reader refuses an unkeyed
        // digest, even one that matches, so a carrier cannot swap a MAC for a
        // digest it computed itself; a keyless reader cannot check a MAC.
        let swapped = keyed
            .restore::<Review, _>(&unkeyed.seal(&review), json.clone())
            .unwrap_err();
        assert_eq!(swapped.kind(), ErrorKind::Integrity);
        let keyless = unkeyed
            .restore::<Review, _>(&keyed.seal(&review), json)
            .unwrap_err();
        assert_eq!(keyless.kind(), ErrorKind::Configuration);
    }

    /// A report and a review never seal alike, whatever their fields, and a
    /// report's digest is what its unkeyed seal records.
    #[test]
    fn a_seal_names_the_kind_of_value() {
        let report = Report::new();
        let review = Review::new(&report);
        assert_ne!(Seal::new(&report, None), Seal::new(&review, None));
        assert_eq!(Seal::new(&report, None), Seal::Digest(report.digest()));
    }
}
