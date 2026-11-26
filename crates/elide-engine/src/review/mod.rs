//! [`Review`]: a reviewer's edits to a [`Report`], made against one version
//! of it, and the [`ReviewPolicy`] that decides which are accepted.
//!
//! A review is the only way to change what a report says. It names the report
//! it was made against by digest, carries edits rather than an edited report,
//! and is applied by [`Orchestrator::review`], which checks each edit against
//! the policy and records it on the entities it touches: an include as the
//! event that starts its entity, every other edit as a `Manual` decision. So a
//! review layer that never holds the report's authority, a browser, a review
//! queue, can still say what it wants changed, and the trusted side decides
//! what it accepts.
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
pub(crate) use self::group::{EditGroup, Reviewer};
pub use self::policy::{
    And, Decision, Narrowed, ProposedEdit, Protect, RequireReason, ReviewPolicy,
};
use crate::wire::{digest, part_fields};
use crate::{PartId, Report};

/// A reviewer's edits to a [`Report`], made against one version of it.
///
/// Start one from the report a reviewer is shown with [`new`](Self::new), add
/// edits part by part through [`part`](Self::part), and apply it with
/// [`Orchestrator::review`]. It serializes, so it can travel from a review
/// layer to the side that holds the report, and is read back with a
/// [`Restorer`].
///
/// A review is bound to the [digest](Report::digest) of the report it was made
/// against, and applies only to that report: one made against another
/// report, or an earlier version of this one, is refused. A review that
/// changes the report changes its digest, so it applies once.
///
/// A review carries what was decided and why, not who decided it: the side
/// that applies it names the actor, from however it authenticated the
/// reviewer, so a review cannot claim to be someone else's. What a review
/// cannot protect itself against is its carrier: an edit dropped on the way
/// leaves the entity as the analysis found it, and an
/// [include](ReviewPart::include) dropped leaves content unredacted. Carry it
/// over a channel the deployment authenticates.
///
/// [`Restorer`]: crate::Restorer
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

    /// A digest of the review: what a [`Restorer`](crate::Restorer) checks it
    /// against when it is read back.
    #[must_use]
    pub fn digest(&self) -> AuditHash {
        digest(self)
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
    /// review already holds edits of part `id` as another modality. A part
    /// opened as another modality but left without edits is reopened as `M`.
    pub fn part<M: Modality>(&mut self, id: &PartId) -> Result<ReviewPart<'_, M>>
    where
        M::Location: Serialize + DeserializeOwned,
    {
        let index = match self.parts.iter().position(|(part, _)| part == id) {
            Some(index) if self.parts[index].1.is_empty() => {
                self.parts[index].1 = Box::new(Vec::<Edit<M>>::new());
                index
            }
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

    /// Apply the review to `report` as made by the `reviewer`: every edit or
    /// none.
    ///
    /// The review must have been made against `report` as it is now. Each
    /// part's edits are checked and applied in turn on a copy, and the copies
    /// replace the parts only once every edit passed, so a refused edit leaves
    /// the report as it was.
    pub(crate) fn apply(&self, report: &mut Report, reviewer: Reviewer<'_>) -> Result<()> {
        if reviewer.by.trim().is_empty() {
            return Err(Error::malformed("a review is applied by a named actor"));
        }
        if self.against != report.digest() {
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
        // A group with no edits is one the digest leaves out, and so is skipped
        // before its modality is matched against the part's.
        for (id, edits) in self.parts.iter().filter(|(_, edits)| !edits.is_empty()) {
            // A part with no entities is one the digest leaves out, so it is
            // edited as if absent, whatever modality it was held as.
            let empty;
            let entities = match report.parts.get(id) {
                Some(entities) if !entities.is_empty() => entities.as_ref(),
                _ => {
                    empty = edits.empty_entities();
                    empty.as_ref()
                }
            };
            reviewed.push((id.clone(), edits.apply(id, entities, reviewer)?));
        }
        report.parts.extend(reviewed);
        Ok(())
    }

    /// The fields the review's digest covers: the report's digest, then each
    /// part by path with its modality and its edits in order. Parts are
    /// ordered by path, as they apply independently.
    pub(crate) fn digest_fields(&self) -> Vec<Vec<u8>> {
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
/// Returned by [`Review::part`]. Each edit carries the reason set with
/// [`because`](Self::because), which the [`ReviewPolicy`] may require, and
/// which the applied report records on the entities the edit touches.
#[derive(Debug)]
pub struct ReviewPart<'r, M: Modality> {
    edits: &'r mut Vec<Edit<M>>,
    because: Option<Attribution>,
}

impl<M: Modality> ReviewPart<'_, M> {
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

    /// Redact the entity `entity`, a finding the reviewer confirms, even
    /// after an earlier suppression. Confirming an entity no reviewer has
    /// suppressed records the confirmation and changes no redaction.
    pub fn confirm(&mut self, entity: Uuid) -> &mut Self {
        self.push(Change::Confirm { entity })
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

    /// Add `change`, with the reason this view gives.
    fn push(&mut self, change: Change<M>) -> &mut Self {
        self.edits.push(Edit {
            change,
            because: self.because.clone(),
        });
        self
    }
}

#[cfg(test)]
mod tests {
    use std::ops::Range;

    use elide_core::ErrorKind;
    use elide_core::entity::Entity;
    use elide_core::entity::audit::{AuditKind, Manual, ManualIntent};
    use elide_core::modality::metadata::Metadata;
    use elide_core::modality::text::{Text, TextLocation};
    use elide_core::primitive::Confidence;

    use super::*;
    use crate::Restorer;
    use crate::analysis::test_support::{doc, text_entity, text_entity_at as at};

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

    /// Apply `review` to `report` as `reviewer-7`, under a policy that
    /// accepts every edit.
    fn apply(report: &mut Report, review: &Review) -> Result<()> {
        review.apply(
            report,
            Reviewer {
                policy: &|_: &ProposedEdit<'_>| Ok(()),
                by: "reviewer-7",
            },
        )
    }

    #[test]
    fn an_included_entity_is_a_reviewers_assertion() {
        let mut report = Report::new();
        let mut review = Review::new(&report);
        let id = review
            .part::<Text>(&doc())
            .unwrap()
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
        let [birth] = entity.audit().events() else {
            panic!("the inclusion is the entity's only event");
        };
        let AuditKind::Included(included) = &birth.kind else {
            panic!("the entity starts from an inclusion");
        };
        assert_eq!(birth.source, "reviewer-7");
        assert_eq!(included.location, TextLocation::inline(4..12));
        assert_eq!(
            included.attribution,
            Some(Attribution::freeform("missed by detection").into())
        );
        assert!(decisions(entity).is_empty());
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

    #[test]
    fn a_refusal_in_a_later_part_rolls_back_an_earlier_part() {
        let nested = doc().child("nested.txt");
        let first = text_entity("EMAIL_ADDRESS");
        let protected = text_entity("SSN");
        let (first_id, protected_id) = (first.id(), protected.id());
        let mut report = Report::new()
            .insert_part::<Text>(doc(), vec![first])
            .insert_part::<Text>(nested.clone(), vec![protected]);
        let before = report.digest();
        let mut review = Review::new(&report);
        review.part::<Text>(&doc()).unwrap().suppress(first_id);
        review.part::<Text>(&nested).unwrap().suppress(protected_id);

        let err = review
            .apply(
                &mut report,
                Reviewer {
                    policy: &Protect::labels(["SSN"]),
                    by: "reviewer-7",
                },
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Refused);
        assert_eq!(report.digest(), before);
        assert!(!report.part::<Text>(&doc()).unwrap().entities()[0].is_suppressed());

        // The same review can still be applied when both edits are accepted.
        apply(&mut report, &review).unwrap();
        for part in [doc(), nested] {
            assert!(report.part::<Text>(&part).unwrap().entities()[0].is_suppressed());
        }
    }

    #[test]
    fn a_review_cannot_be_applied_to_a_different_report_with_identical_findings() {
        let entity = text_entity("EMAIL_ADDRESS");
        let id = entity.id();
        let original = Report::new().insert_part::<Text>(doc(), vec![entity.clone()]);
        let mut other = Report::new().insert_part::<Text>(doc(), vec![entity]);
        let mut review = Review::new(&original);
        review.part::<Text>(&doc()).unwrap().suppress(id);

        let err = apply(&mut other, &review).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Integrity);
        assert!(!other.part::<Text>(&doc()).unwrap().entities()[0].is_suppressed());
    }

    #[test]
    fn include_ids_cannot_be_reused_across_parts() {
        let existing = text_entity("EMAIL_ADDRESS");
        let existing_id = existing.id();
        let nested = doc().child("nested.txt");
        let mut report = Report::new().insert_part::<Text>(doc(), vec![existing]);
        let before = report.digest();
        let mut review = Review::new(&report);
        let new_id = review
            .part::<Text>(&doc())
            .unwrap()
            .include("PHONE_NUMBER", TextLocation::inline(10..20));
        review
            .part::<Text>(&nested)
            .unwrap()
            .include("SSN", TextLocation::inline(0..5));
        let json = serde_json::to_value(&review).unwrap();
        let restorer = Restorer::new().with_modality::<Text>();
        let nested_path = serde_json::json!(nested.segments().collect::<Vec<_>>());

        for duplicate in [existing_id, new_id] {
            let mut wire = json.clone();
            let parts = wire["parts"].as_array_mut().unwrap();
            let nested_edits = parts
                .iter_mut()
                .find(|part| part["id"] == nested_path)
                .unwrap();
            nested_edits["edits"][0]["entity"] = serde_json::json!(duplicate);
            let loaded: Review = restorer.restore_unchecked(wire).unwrap();

            let err = apply(&mut report, &loaded).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::MalformedInput);
            assert_eq!(report.digest(), before);
            assert!(report.part::<Text>(&nested).is_err());
        }
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
            review
                .apply(
                    &mut report,
                    Reviewer {
                        policy: &policy,
                        by: "reviewer-7",
                    },
                )
                .map_err(|err| err.kind())
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
        assert_eq!(
            outcome(&|part| {
                part.confirm(id);
            }),
            Ok(()),
        );
    }

    /// An edit narrows the entities overlapping what it touches, as they share
    /// one redaction: suppressing a neighbour of a protected entity is
    /// refused, and one clear of it is not.
    #[test]
    fn an_edit_narrows_the_entities_it_overlaps() {
        let outcome = |at_: Range<usize>| {
            let email = at("EMAIL_ADDRESS", at_);
            let id = email.id();
            let mut report =
                Report::new().insert_part::<Text>(doc(), vec![email, at("SSN", 10..16)]);
            let mut review = Review::new(&report);
            review.part::<Text>(&doc()).unwrap().suppress(id);
            review
                .apply(
                    &mut report,
                    Reviewer {
                        policy: &Protect::labels(["SSN"]),
                        by: "reviewer-7",
                    },
                )
                .map_err(|err| err.kind())
        };

        assert_eq!(outcome(0..20), Err(ErrorKind::Refused));
        assert_eq!(outcome(30..40), Ok(()));
    }

    /// An edit narrows its whole overlap cluster: suppressing an entity that
    /// reaches a protected one only through a neighbour is refused too.
    #[test]
    fn an_edit_narrows_its_whole_overlap_cluster() {
        let first = at("EMAIL_ADDRESS", 0..5);
        let id = first.id();
        let between = at("PERSON_NAME", 4..9);
        let protected = at("SSN", 8..13);
        let mut report = Report::new().insert_part::<Text>(doc(), vec![first, between, protected]);
        let mut review = Review::new(&report);
        review.part::<Text>(&doc()).unwrap().suppress(id);

        let reviewer = Reviewer {
            policy: &Protect::labels(["SSN"]),
            by: "reviewer-7",
        };
        let err = review.apply(&mut report, reviewer).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Refused);
    }

    #[test]
    fn relocation_checks_the_destination_cluster_without_crossing_suppressed_entities() {
        let moving = at("EMAIL_ADDRESS", 0..5);
        let moving_id = moving.id();
        let bridge = at("PERSON_NAME", 12..18);
        let bridge_id = bridge.id();
        let protected = at("SSN", 17..22);
        let mut report = Report::new().insert_part::<Text>(doc(), vec![moving, bridge, protected]);
        let destination = TextLocation::inline(8..14);
        let reviewer = Reviewer {
            policy: &Protect::labels(["SSN"]),
            by: "reviewer-7",
        };
        let relocation = |report: &Report| {
            let mut review = Review::new(report);
            review
                .part::<Text>(&doc())
                .unwrap()
                .relocate(moving_id, destination.clone());
            review
        };

        let err = relocation(&report)
            .apply(&mut report, reviewer)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Refused);

        let mut suppress_bridge = Review::new(&report);
        suppress_bridge
            .part::<Text>(&doc())
            .unwrap()
            .suppress(bridge_id);
        apply(&mut report, &suppress_bridge).unwrap();

        relocation(&report).apply(&mut report, reviewer).unwrap();
        let part = report.part::<Text>(&doc()).unwrap();
        let moved = part
            .entities()
            .iter()
            .find(|e| e.id() == moving_id)
            .unwrap();
        assert_eq!(moved.location(), &destination);
    }

    /// Moving a suppressed entity changes no redaction, so it narrows nothing,
    /// even onto a protected one.
    #[test]
    fn moving_a_suppressed_entity_narrows_nothing() {
        let mut spared = at("EMAIL_ADDRESS", 0..5);
        let id = spared.id();
        let location = spared.location().clone();
        spared.record_manual(Manual::new(ManualIntent::Suppress, location), "reviewer-7");
        let mut report = Report::new().insert_part::<Text>(doc(), vec![spared, at("SSN", 20..26)]);
        let mut review = Review::new(&report);
        review
            .part::<Text>(&doc())
            .unwrap()
            .relocate(id, TextLocation::inline(18..28));

        let reviewer = Reviewer {
            policy: &Protect::labels(["SSN"]),
            by: "reviewer-7",
        };
        review.apply(&mut report, reviewer).unwrap();
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
            Decision::Relocate => Ok(()),
            _ => protect.check(edit),
        };
        let err = review
            .apply(
                &mut report,
                Reviewer {
                    policy: &policy,
                    by: "reviewer-7",
                },
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Refused);

        let mut unmoved = Review::new(&report);
        unmoved
            .part::<Text>(&doc())
            .unwrap()
            .suppress_within(TextLocation::inline(0..20));
        unmoved
            .apply(
                &mut report,
                Reviewer {
                    policy: &policy,
                    by: "reviewer-7",
                },
            )
            .expect("unmoved, the SSN lies outside the region, so nothing narrows it");
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

    /// The actor is the one the review is applied by, and must be named.
    #[test]
    fn a_review_is_applied_by_a_named_actor() {
        let entity = text_entity("EMAIL_ADDRESS");
        let id = entity.id();
        let mut report = Report::new().insert_part::<Text>(doc(), vec![entity]);
        let mut review = Review::new(&report);
        review.part::<Text>(&doc()).unwrap().suppress(id);
        let accept = |_: &ProposedEdit<'_>| Ok(());

        let err = review
            .apply(
                &mut report,
                Reviewer {
                    policy: &accept,
                    by: " ",
                },
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::MalformedInput);
        review
            .apply(
                &mut report,
                Reviewer {
                    policy: &accept,
                    by: "lead-2",
                },
            )
            .unwrap();
        let entity = report.entity_anywhere::<Text>(id).unwrap();
        assert_eq!(decisions(entity)[0].0, "lead-2");
    }

    #[test]
    fn a_policy_can_require_a_reason_to_narrow() {
        let entity = text_entity("EMAIL_ADDRESS");
        let id = entity.id();
        let outcome = |because: Option<&str>| {
            let mut report = Report::new().insert_part::<Text>(doc(), vec![entity.clone()]);
            let mut review = Review::new(&report);
            let mut part = review.part::<Text>(&doc()).unwrap();
            if let Some(because) = because {
                part = part.because(Attribution::freeform(because));
            }
            part.suppress(id);
            review
                .apply(
                    &mut report,
                    Reviewer {
                        policy: &RequireReason,
                        by: "reviewer-7",
                    },
                )
                .map_err(|err| err.kind())
        };

        assert_eq!(outcome(None), Err(ErrorKind::Refused));
        assert_eq!(outcome(Some("public")), Ok(()));
    }

    /// A confirmation takes back an earlier suppression: the latest verdict on
    /// the entity's own trail decides whether it is redacted.
    #[test]
    fn a_confirmation_undoes_a_suppression() {
        let entity = at("EMAIL_ADDRESS", 4..10);
        let id = entity.id();
        let location = entity.location().clone();
        let mut report = Report::new().insert_part::<Text>(doc(), vec![entity]);
        let verdict = |report: &mut Report, edit: &dyn Fn(&mut ReviewPart<'_, Text>)| {
            let mut review = Review::new(report);
            edit(&mut review.part::<Text>(&doc()).unwrap());
            apply(report, &review).unwrap();
            report.entity_anywhere::<Text>(id).unwrap().is_suppressed()
        };

        assert!(verdict(&mut report, &|part| {
            part.suppress(id);
        }));
        assert!(!verdict(&mut report, &|part| {
            part.confirm(id);
        }));
        assert!(
            verdict(&mut report, &|part| {
                part.confirm(id).suppress(id);
            }),
            "the latest verdict wins",
        );

        let entity = report.entity_anywhere::<Text>(id).unwrap();
        let decision = |intent| ("reviewer-7".to_owned(), intent, location.clone());
        assert_eq!(
            decisions(entity),
            [
                decision(ManualIntent::Suppress),
                decision(ManualIntent::Confirm),
                decision(ManualIntent::Suppress),
            ],
            "a verdict that is already the latest records nothing",
        );
        assert!(entity.audit().verify().is_ok());
    }

    /// Confirming an entity no reviewer suppressed records the confirmation
    /// once and changes no redaction.
    #[test]
    fn confirming_an_unsuppressed_entity_records_it_once() {
        let entity = at("EMAIL_ADDRESS", 4..10);
        let id = entity.id();
        let mut report = Report::new().insert_part::<Text>(doc(), vec![entity]);
        let mut review = Review::new(&report);
        review.part::<Text>(&doc()).unwrap().confirm(id).confirm(id);
        apply(&mut report, &review).unwrap();

        let entity = report.entity_anywhere::<Text>(id).unwrap();
        assert!(!entity.is_suppressed());
        assert_eq!(entity.audit().verdict(), Some(&ManualIntent::Confirm));
        assert_eq!(decisions(entity).len(), 1);
    }

    /// A confirmation names an entity the part must hold.
    #[test]
    fn confirming_a_missing_entity_is_refused() {
        let mut report = Report::new().insert_part::<Text>(doc(), vec![at("SSN", 4..10)]);
        let mut review = Review::new(&report);
        review.part::<Text>(&doc()).unwrap().confirm(Uuid::now_v7());
        let err = apply(&mut report, &review).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
    }

    /// A review that changes nothing applies, and changes nothing: applying it
    /// again is as harmless as the first time.
    #[test]
    fn a_review_that_changes_nothing_is_harmless() {
        let entity = at("EMAIL_ADDRESS", 30..40);
        let mut report = Report::new().insert_part::<Text>(doc(), vec![entity]);
        let mut review = Review::new(&report);
        review
            .part::<Text>(&doc())
            .unwrap()
            .suppress_within(TextLocation::inline(0..10));

        let before = report.digest();
        apply(&mut report, &review).unwrap();
        apply(&mut report, &review).unwrap();
        assert_eq!(report.digest(), before);
    }

    /// An entity already suppressed is not redacted, so no edit narrows it,
    /// even under a policy that protects its label.
    #[test]
    fn an_already_suppressed_entity_is_not_narrowed() {
        let mut ssn = at("SSN", 4..10);
        let id = ssn.id();
        let location = ssn.location().clone();
        ssn.record_manual(Manual::new(ManualIntent::Suppress, location), "reviewer");
        let mut report = Report::new().insert_part::<Text>(doc(), vec![ssn]);
        let mut review = Review::new(&report);
        review
            .part::<Text>(&doc())
            .unwrap()
            .relabel(id, "PUBLIC_ID");

        review
            .apply(
                &mut report,
                Reviewer {
                    policy: &Protect::labels(["SSN"]),
                    by: "reviewer-7",
                },
            )
            .unwrap();
        assert_eq!(
            *report.entity_anywhere::<Text>(id).unwrap().label(),
            LabelRef::new("PUBLIC_ID")
        );
    }

    /// A part opened as another modality but left without edits is reopened
    /// as the one asked for; one holding edits is not.
    #[test]
    fn an_empty_part_is_reopened_as_another_modality() {
        let report = Report::new();
        let mut review = Review::new(&report);
        review.part::<Metadata>(&doc()).unwrap();
        review
            .part::<Text>(&doc())
            .unwrap()
            .include("SSN", TextLocation::inline(0..4));
        let err = review.part::<Metadata>(&doc()).map(drop).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::MalformedInput);
    }

    /// A part with no entities is edited as if absent, whatever modality it
    /// was held as, as the digest leaves it out.
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
            if edit.narrows() && edit.by != "lead" {
                return Err(Error::refused("only a lead may narrow"));
            }
            assert!(edit.narrowed.iter().all(|narrowed| narrowed.id == id));
            Ok(())
        };
        let outcome = |by: &str| {
            let mut report = Report::new().insert_part::<Text>(doc(), vec![entity.clone()]);
            let mut review = Review::new(&report);
            let mut part = review.part::<Text>(&doc()).unwrap();
            part.include("PERSON_NAME", TextLocation::inline(8..12));
            part.suppress(id);
            review
                .apply(
                    &mut report,
                    Reviewer {
                        policy: &only_leads_narrow,
                        by,
                    },
                )
                .map_err(|err| err.kind())
        };

        assert_eq!(outcome("reviewer-7"), Err(ErrorKind::Refused));
        assert_eq!(outcome("lead"), Ok(()));
    }

    /// A review reads back against its digest; one with an edit dropped on
    /// the way does not.
    #[test]
    fn a_review_reads_back_only_against_its_digest() {
        let entity = text_entity("EMAIL_ADDRESS");
        let id = entity.id();
        let report = Report::new().insert_part::<Text>(doc(), vec![entity]);
        let mut review = Review::new(&report);
        let mut part = review.part::<Text>(&doc()).unwrap();
        part.suppress(id);
        part.include("PERSON_NAME", TextLocation::inline(8..12));
        let restorer = Restorer::new().with_modality::<Text>();
        let json = serde_json::to_value(&review).unwrap();

        let read: Review = restorer
            .restore(&review.digest(), json.clone())
            .expect("reads back intact");
        assert_eq!(read.digest(), review.digest());
        assert_eq!(read.edits::<Text>(&doc()).unwrap().len(), 2);

        let mut dropped = json;
        dropped["parts"][0]["edits"]
            .as_array_mut()
            .unwrap()
            .remove(1);
        let err = restorer
            .restore::<Review, _>(&review.digest(), dropped)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Integrity);
    }

    /// A report and a review never digest alike, whatever their fields.
    #[test]
    fn a_digest_names_the_kind_of_value() {
        let report = Report::new();
        let review = Review::new(&report);
        assert_ne!(report.digest(), review.digest());
    }
}
