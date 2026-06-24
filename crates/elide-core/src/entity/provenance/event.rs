//! Single [`Event`] in an entity's life, and the [`EventKind`] of event
//! it can be.

use hipstr::HipStr;
use jiff::Timestamp;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::modality::{Hint, Modality};
use crate::primitive::Confidence;
use crate::redaction::{Attribution, LeakProfile, OperatorId, RuleMatch};

/// One thing that happened to an entity, with its effect on confidence.
///
/// Events are recorded in order on an entity's [`Provenance`], forming
/// the full audit trail of its life: each recognizer that found it, the
/// deduplication that fused them, any score calibration, and the
/// redaction that hid it. The uniform spine (who, before/after score,
/// when, why) is the same for every event; the [`kind`] carries the
/// event-specific detail.
///
/// `entity.confidence` always equals the [`after`] of the most recent
/// event.
///
/// [`Provenance`]: crate::entity::provenance::Provenance
/// [`kind`]: Event::kind
/// [`after`]: Event::after
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(bound = "M::Location: Serialize + for<'a> Deserialize<'a>, \
                   M::Data: Serialize + for<'a> Deserialize<'a>")
)]
pub struct Event<M: Modality> {
    /// Who produced this event: a recognizer name, a deduplication strategy,
    /// an operator, or whatever acted.
    pub source: HipStr<'static>,
    /// Confidence before this event, if there was a prior value. `None` on
    /// the first (birth) event.
    pub before: Option<Confidence>,
    /// Confidence after this event.
    pub after: Confidence,
    /// When the event happened (UTC).
    pub at: Timestamp,
    /// Free-text explanation of what the event did and why.
    pub reason: HipStr<'static>,
    /// Kind of event, with its event-specific detail.
    pub kind: EventKind<M>,
}

impl<M: Modality> Event<M> {
    /// Recognition event from a pattern/dictionary recognizer.
    pub fn pattern(
        source: impl Into<HipStr<'static>>,
        confidence: Confidence,
        location: M::Location,
        pattern: PatternEvent,
    ) -> Self {
        Self {
            source: source.into(),
            before: None,
            after: confidence,
            at: Timestamp::now(),
            reason: HipStr::default(),
            kind: EventKind::Pattern { location, pattern },
        }
    }

    /// Recognition event from a model/NER recognizer.
    pub fn model(
        source: impl Into<HipStr<'static>>,
        confidence: Confidence,
        location: M::Location,
        model: ModelEvent,
    ) -> Self {
        Self {
            source: source.into(),
            before: None,
            after: confidence,
            at: Timestamp::now(),
            reason: HipStr::default(),
            kind: EventKind::Model { location, model },
        }
    }

    /// Deduplication (fusion) event combining several detections.
    pub fn deduplication(
        strategy: impl Into<HipStr<'static>>,
        before: Confidence,
        after: Confidence,
    ) -> Self {
        let strategy = strategy.into();
        Self {
            source: strategy.clone(),
            before: Some(before),
            after,
            at: Timestamp::now(),
            reason: HipStr::default(),
            kind: EventKind::Deduplication { strategy },
        }
    }

    /// Calibration event rescaling confidence by `factor`.
    pub fn calibration(before: Confidence, after: Confidence, factor: f64) -> Self {
        Self {
            source: HipStr::borrowed("calibration"),
            before: Some(before),
            after,
            at: Timestamp::now(),
            reason: HipStr::default(),
            kind: EventKind::Calibration { factor },
        }
    }

    /// Refinement event: a context keyword near the entity lifted its
    /// confidence.
    ///
    /// `location` is where the boosting keyword sits in the medium: for a
    /// hint match the hint's own location, for an in-text-window match the
    /// keyword resolved through the modality (`None` if it couldn't be
    /// placed).
    pub fn refinement(
        source: impl Into<HipStr<'static>>,
        before: Confidence,
        after: Confidence,
        keyword: impl Into<HipStr<'static>>,
        hint: Option<Hint<M>>,
        location: Option<M::Location>,
    ) -> Self {
        Self {
            source: source.into(),
            before: Some(before),
            after,
            at: Timestamp::now(),
            reason: HipStr::default(),
            kind: EventKind::Refinement {
                keyword: keyword.into(),
                hint,
                location,
            },
        }
    }

    /// Redaction event hiding the entity with `operator`.
    pub fn redaction(
        operator: OperatorId,
        leak_profile: LeakProfile,
        confidence: Confidence,
        matched_by: RuleMatch,
        attribution: Option<Attribution>,
    ) -> Self {
        let source = operator.name.clone();
        Self {
            source,
            before: Some(confidence),
            after: confidence,
            at: Timestamp::now(),
            reason: HipStr::default(),
            kind: EventKind::Redaction {
                operator,
                leak_profile,
                key_id: None,
                matched_by,
                attribution,
            },
        }
    }

    /// Attach a free-text reason, consuming and returning `self`.
    #[must_use]
    pub fn with_reason(mut self, reason: impl Into<HipStr<'static>>) -> Self {
        self.reason = reason.into();
        self
    }

    /// Whether this event is a recognition (pattern or model).
    pub fn is_recognition(&self) -> bool {
        matches!(
            self.kind,
            EventKind::Pattern { .. } | EventKind::Model { .. }
        )
    }
}

/// Kind of an [`Event`], carrying its event-specific detail.
///
/// `#[non_exhaustive]`: new event kinds (verification, annotation, …)
/// can be added compatibly. The recognition kinds ([`Pattern`],
/// [`Model`]) carry the matched [`Location`]; the rest carry their own
/// data.
///
/// [`Pattern`]: EventKind::Pattern
/// [`Model`]: EventKind::Model
/// [`Location`]: Modality::Location
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(
        tag = "kind",
        rename_all = "snake_case",
        bound = "M::Location: Serialize + for<'a> Deserialize<'a>, \
                 M::Data: Serialize + for<'a> Deserialize<'a>"
    )
)]
#[non_exhaustive]
pub enum EventKind<M: Modality> {
    /// A pattern or dictionary recognizer matched here.
    Pattern {
        /// Where the recognizer matched.
        location: M::Location,
        /// Pattern detail.
        pattern: PatternEvent,
    },
    /// A model / NER recognizer matched here.
    Model {
        /// Where the recognizer matched.
        location: M::Location,
        /// Model detail.
        model: ModelEvent,
    },
    /// Several detections were fused into one entity.
    Deduplication {
        /// Name of the fusion strategy that combined them.
        strategy: HipStr<'static>,
    },
    /// The entity's confidence was rescaled by a per-recognizer factor.
    Calibration {
        /// Multiplier applied.
        factor: f64,
    },
    /// A context keyword near the entity lifted its confidence.
    Refinement {
        /// Keyword that fired the boost.
        keyword: HipStr<'static>,
        /// The located [`Hint`] the keyword fired from, when the match came
        /// from an out-of-band hint (a column header, a key) rather than
        /// the in-text word window. `None` for an in-text-window match.
        ///
        /// [`Hint`]: crate::modality::Hint
        hint: Option<Hint<M>>,
        /// Where the boosting keyword sits in the medium. For a hint match
        /// this mirrors the hint's own location; for an in-text-window match
        /// it is the keyword resolved through the modality's [`locate`] (a
        /// pixel box for image, a time span for audio, the byte range for
        /// text/tabular). `None` when the keyword's stream range could not be
        /// placed — symmetric with a match the recognizer itself drops.
        ///
        /// [`locate`]: crate::modality::TextRecognizable::locate
        location: Option<M::Location>,
    },
    /// An operator hid the entity.
    Redaction {
        /// Which operator (name + version) ran.
        operator: OperatorId,
        /// How much the output leaks about the original.
        leak_profile: LeakProfile,
        /// Identifier of the key needed to reverse it, if reversible.
        key_id: Option<HipStr<'static>>,
        /// Which selection rule chose this operator — the automatic "why"
        /// (matched a label, a tag, a predicate, or the fallback).
        matched_by: RuleMatch,
        /// The author-supplied policy rationale, when the operator carried an
        /// [`Attribution`]; `None` otherwise.
        attribution: Option<Attribution>,
    },
}

/// Detail of a pattern/dictionary recognition.
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct PatternEvent {
    /// Name of the pattern that matched (e.g. `"ssn"`, `"email"`).
    pub name: HipStr<'static>,
    /// Literal regex source that matched, when exposed.
    pub regex: Option<HipStr<'static>>,
    /// Name of the validator that confirmed the match (e.g. `"luhn"`).
    pub validator: Option<HipStr<'static>>,
    /// Whether contextual analysis (keyword co-occurrence) adjusted the
    /// score for this match.
    pub contextual: bool,
}

/// Detail of a model/NER recognition.
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ModelEvent {
    /// Model name (e.g. `"spacy-en-core-web-lg"`, `"gpt-4"`).
    pub name: HipStr<'static>,
    /// Model version string, when known.
    pub version: Option<HipStr<'static>>,
    /// Whether contextual analysis adjusted the score for this match.
    pub contextual: bool,
}
