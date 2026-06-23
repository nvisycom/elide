//! [`EntityDraft`]: a recognizer's stream-positioned finding, before it is
//! lifted to a native modality location.

use std::ops::Range;

use elide_core::entity::provenance::PatternEvent;
use elide_core::entity::{EntityCoRef, LabelRef};
use elide_core::primitive::Confidence;
use hipstr::HipStr;

/// A [`StreamRecognizer`] match, positioned by a stream byte range.
///
/// Not yet placed in the medium; that happens at [`lift`].
///
/// Deliberately **non-generic**. A draft's position is a range into the
/// `as_text` `&str`, which is the same shape for every modality (a byte
/// stream); the modality enters only when [`lift`] turns the
/// [`stream_range`] into a native location. Keeping the draft modality-free
/// is what lets the keyword-boost [`Enhancer`] operate on it uniformly.
///
/// The `stream_range` is an implementation detail of whichever OCR/STT
/// engine produced the text and is **ephemeral**: it lives only between
/// [`find`] and [`lift`], and never reaches the serialized [`Entity`].
///
/// [`StreamRecognizer`]: super::StreamRecognizer
/// [`lift`]: super::lift
/// [`stream_range`]: Self::stream_range
/// [`find`]: super::StreamRecognizer::find
/// [`Enhancer`]: crate::Enhancer
/// [`Entity`]: elide_core::entity::Entity
#[derive(Debug, Clone)]
pub struct EntityDraft {
    /// The label the recognizer assigned.
    pub label: LabelRef,
    /// The match's confidence, mutated in place by the [`Enhancer`].
    ///
    /// [`Enhancer`]: crate::Enhancer
    pub confidence: Confidence,
    /// Byte range of the match in the recognized-text stream. Ephemeral:
    /// consumed by [`lift`], never stored on the [`Entity`].
    ///
    /// [`lift`]: super::lift
    /// [`Entity`]: elide_core::entity::Entity
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
/// fills in.
///
/// [`Event`]: elide_core::entity::provenance::Event
/// [`lift`]: super::lift
#[derive(Debug, Clone)]
pub struct DraftEvent {
    /// Event source tag (e.g. `"pattern"`).
    pub source: HipStr<'static>,
    /// Free-text reason for the match.
    pub reason: HipStr<'static>,
    /// Pattern/dictionary metadata stamped onto the birth event.
    pub pattern: PatternEvent,
}
