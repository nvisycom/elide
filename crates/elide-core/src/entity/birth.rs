//! [`Birth`]: the event an [`Entity`](super::Entity) starts from, and what it
//! can be.

use uuid::Uuid;

use super::audit::{AuditEvent, AuditKind, Flagged, Included, Model, Pattern};
use super::state::State;
use crate::modality::Modality;
use crate::primitive::Confidence;

/// The event an [`Entity`](super::Entity) starts from: how sure its finder is,
/// and what it found (which names who found it), with the entity's identity.
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
    /// The birth, at `confidence`, of what `kind` found.
    pub fn new(confidence: Confidence, kind: impl Into<BirthKind<M>>) -> Self {
        let event = AuditEvent::new(confidence, kind.into().into_kind());
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
    Included(Included<M>),
}

impl<M: Modality> BirthKind<M> {
    /// The audit kind recording what was found.
    fn into_kind(self) -> AuditKind<M> {
        match self {
            Self::Pattern(p) => p.into(),
            Self::Model(m) => m.into(),
            Self::Flagged(f) => f.into(),
            Self::Included(i) => i.into(),
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

impl<M: Modality> From<Included<M>> for BirthKind<M> {
    fn from(include: Included<M>) -> Self {
        Self::Included(include)
    }
}
