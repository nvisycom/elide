//! [`Birth`]: the event an [`Entity`](super::Entity) starts from, and what it
//! can be.

use hipstr::HipStr;
use uuid::Uuid;

use super::LabelRef;
use super::audit::{
    Attribution, AuditEvent, AuditKind, Flagged, Manual, ManualIntent, Model, Pattern,
};
use super::state::State;
use crate::modality::Modality;
use crate::primitive::Confidence;

/// The event an [`Entity`](super::Entity) starts from: who found it, how sure
/// they are, and what they found, with the entity's identity.
///
/// Only a recognition or a reviewer's inclusion starts an entity, so a birth
/// holds only one of those, as its [`BirthKind`]. Its id is a fresh
/// time-ordered UUIDv7, or the one [`with_id`](Self::with_id) gives.
#[derive(Debug, Clone)]
pub struct Birth<M: Modality> {
    pub(super) id: Uuid,
    pub(super) state: State<M>,
    pub(super) event: AuditEvent<M>,
}

impl<M: Modality> Birth<M> {
    /// The birth `source` records, at `confidence`, of what `kind` found.
    pub fn new(
        source: impl Into<HipStr<'static>>,
        confidence: Confidence,
        kind: impl Into<BirthKind<M>>,
    ) -> Self {
        let event = AuditEvent::new(source, confidence, kind.into().into_kind());
        let state = State::born(&event).expect("a recognition or an inclusion starts an entity");
        Self {
            id: Uuid::now_v7(),
            state,
            event,
        }
    }

    /// Give the entity `id`, one chosen beforehand, such as the id a review
    /// gave the entity it includes.
    #[must_use]
    pub fn with_id(mut self, id: Uuid) -> Self {
        self.id = id;
        self
    }
}

/// What a [`Birth`] found: a recognition, or a reviewer's inclusion.
#[derive(Debug, Clone)]
pub enum BirthKind<M: Modality> {
    /// A pattern or dictionary recognizer matched.
    Pattern(Pattern<M>),
    /// A model or NER recognizer matched.
    Model(Model<M>),
    /// A format flagged an item of hidden content.
    Flagged(Flagged<M>),
    /// A reviewer included what detection missed.
    Include(Include<M>),
}

impl<M: Modality> BirthKind<M> {
    /// The audit kind recording what was found.
    fn into_kind(self) -> AuditKind<M> {
        match self {
            Self::Pattern(p) => p.into(),
            Self::Model(m) => m.into(),
            Self::Flagged(f) => f.into(),
            Self::Include(include) => Manual {
                intent: ManualIntent::Include {
                    label: include.label,
                },
                location: include.location,
                attribution: include.attribution,
            }
            .into(),
        }
    }
}

impl<M: Modality> From<Pattern<M>> for BirthKind<M> {
    fn from(pattern: Pattern<M>) -> Self {
        Self::Pattern(pattern)
    }
}

impl<M: Modality> From<Model<M>> for BirthKind<M> {
    fn from(model: Model<M>) -> Self {
        Self::Model(model)
    }
}

impl<M: Modality> From<Flagged<M>> for BirthKind<M> {
    fn from(flagged: Flagged<M>) -> Self {
        Self::Flagged(flagged)
    }
}

impl<M: Modality> From<Include<M>> for BirthKind<M> {
    fn from(include: Include<M>) -> Self {
        Self::Include(include)
    }
}

/// A reviewer's inclusion: `label` at `location`, one detection missed,
/// recorded as an [`Include`](ManualIntent::Include) decision.
#[derive(Debug, Clone)]
pub struct Include<M: Modality> {
    /// The label of what was included.
    pub label: LabelRef,
    /// Where it sits.
    pub location: M::Location,
    /// Why it was included.
    pub attribution: Option<Attribution>,
}

impl<M: Modality> Include<M> {
    /// An inclusion of `label` at `location`, with no reason given.
    pub fn new(label: impl Into<LabelRef>, location: M::Location) -> Self {
        Self {
            label: label.into(),
            location,
            attribution: None,
        }
    }

    /// Give the reason for the inclusion.
    #[must_use]
    pub fn because(mut self, attribution: impl Into<Attribution>) -> Self {
        self.attribution = Some(attribution.into());
        self
    }
}
