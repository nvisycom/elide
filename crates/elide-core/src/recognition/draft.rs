//! [`EntityDraft`]: a recognizer's stream-positioned finding, before it is
//! lifted to a native modality location.

use std::ops::Range;

use crate::entity::provenance::{ModelEvent, PatternEvent};
use crate::entity::{EntityCoRef, LabelRef};
use crate::primitive::Confidence;
use hipstr::HipStr;

/// A [`StreamRecognizer`] match, positioned by a stream byte range.
///
/// Not yet placed in the medium; that happens at [`lift`].
///
/// Deliberately **non-generic**. A draft's position is a range into the
/// `as_text` `&str`, which is the same shape for every modality (a byte
/// stream); the modality enters only when [`lift`] turns the
/// [`stream_range`] into a native location. Keeping the draft modality-free
/// is what lets the keyword-boost enhancer operate on it uniformly.
///
/// The `stream_range` is an implementation detail of whichever OCR/STT
/// engine produced the text and is **ephemeral**: it lives only between
/// [`find`] and [`lift`], and never reaches the serialized [`Entity`].
///
/// [`StreamRecognizer`]: super::StreamRecognizer
/// [`lift`]: super::lift
/// [`stream_range`]: Self::stream_range
/// [`find`]: super::StreamRecognizer::find
/// [`Entity`]: crate::entity::Entity
#[derive(Debug, Clone)]
pub struct EntityDraft {
    /// The label the recognizer assigned.
    pub label: LabelRef,
    /// The match's confidence, mutated in place by the keyword-boost
    /// enhancer.
    pub confidence: Confidence,
    /// Byte range of the match in the recognized-text stream. Ephemeral:
    /// consumed by [`lift`], never stored on the [`Entity`].
    ///
    /// [`lift`]: super::lift
    /// [`Entity`]: crate::entity::Entity
    pub stream_range: Range<usize>,
    /// Coreference cluster id, when the recognizer grouped mentions.
    pub coref: Option<EntityCoRef>,
    /// The birth event's pre-lift parts: the native location is added by
    /// [`lift`].
    ///
    /// [`lift`]: super::lift
    pub event: DraftEvent,
}

impl EntityDraft {
    /// A draft with no coreference cluster.
    pub fn new(
        label: LabelRef,
        confidence: Confidence,
        stream_range: Range<usize>,
        event: DraftEvent,
    ) -> Self {
        Self {
            label,
            confidence,
            stream_range,
            coref: None,
            event,
        }
    }

    /// Attach a coreference cluster id.
    #[must_use]
    pub fn with_coref(mut self, coref: EntityCoRef) -> Self {
        self.coref = Some(coref);
        self
    }
}

/// The recognizer-supplied parts of a draft's birth [`Event`].
///
/// Everything except the native location, which [`lift`] resolves and
/// fills in. The [`kind`] carries the recognizer-specific detail — pattern
/// metadata for a dictionary/regex match, model metadata for an NER/LLM
/// match — so a draft can come from any text-localizable recognizer, not
/// just the pattern engine.
///
/// [`Event`]: crate::entity::provenance::Event
/// [`lift`]: super::lift
/// [`kind`]: DraftEvent::kind
#[derive(Debug, Clone)]
pub struct DraftEvent {
    /// Event source tag (e.g. `"pattern"`, `"ner"`).
    pub source: HipStr<'static>,
    /// Free-text reason for the match.
    pub reason: HipStr<'static>,
    /// The recognizer-specific birth-event detail.
    pub kind: DraftEventKind,
}

impl DraftEvent {
    /// A pattern/dictionary draft event.
    pub fn pattern(
        source: impl Into<HipStr<'static>>,
        reason: impl Into<HipStr<'static>>,
        pattern: PatternEvent,
    ) -> Self {
        Self {
            source: source.into(),
            reason: reason.into(),
            kind: DraftEventKind::Pattern(pattern),
        }
    }

    /// A model (NER / LLM) draft event.
    pub fn model(
        source: impl Into<HipStr<'static>>,
        reason: impl Into<HipStr<'static>>,
        model: ModelEvent,
    ) -> Self {
        Self {
            source: source.into(),
            reason: reason.into(),
            kind: DraftEventKind::Model(model),
        }
    }
}

/// Which kind of recognizer produced a draft — the birth-event detail that
/// [`lift`] turns into the matching [`Pattern`] or [`Model`] event.
///
/// [`lift`]: super::lift
/// [`Pattern`]: crate::entity::provenance::EventKind::Pattern
/// [`Model`]: crate::entity::provenance::EventKind::Model
#[derive(Debug, Clone)]
pub enum DraftEventKind {
    /// A regex / dictionary match: carries [`PatternEvent`] metadata.
    Pattern(PatternEvent),
    /// An NER / LLM model match: carries [`ModelEvent`] metadata.
    Model(ModelEvent),
}
