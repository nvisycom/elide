//! [`AudioLocation`]: a time span within audio content.

use std::cmp::Ordering;

use hipstr::HipStr;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::modality::{ModalityLocation, Overlap};
use crate::primitive::TimeSpan;

/// A [`TimeSpan`] within audio content, with an optional speaker label.
///
/// The time span is the coordinate; ordering and overlap consider only it.
/// The optional [`speaker_id`] is a diarization label,
/// not a coordinate: two utterances from different speakers at the same
/// instant still overlap in time, so the speaker is carried for provenance
/// but excluded from comparison.
///
/// [`speaker_id`]: Self::speaker_id
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct AudioLocation {
    /// Time span the location covers, in the stream's timeline.
    pub span: TimeSpan,
    /// Diarization label of the speaker, when a diarizer assigned one.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub speaker_id: Option<HipStr<'static>>,
}

impl AudioLocation {
    /// Location covering `span`, speaker unset.
    pub fn new(span: TimeSpan) -> Self {
        Self {
            span,
            speaker_id: None,
        }
    }

    /// Location covering `[start_ms, end_ms)` in milliseconds, speaker
    /// unset.
    pub fn from_millis(start_ms: u64, end_ms: u64) -> Self {
        Self::new(TimeSpan::from_millis(start_ms, end_ms))
    }

    /// Attach a diarization speaker label.
    #[must_use]
    pub fn with_speaker_id(mut self, speaker_id: impl Into<HipStr<'static>>) -> Self {
        self.speaker_id = Some(speaker_id.into());
        self
    }

    /// Whether the location's span is empty (zero duration).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.span.is_empty()
    }
}

impl ModalityLocation for AudioLocation {
    fn overlap(&self, other: &Self) -> Overlap {
        // Time-span relationship; the speaker label is ignored, so two
        // speakers talking over each other still overlap.
        self.span.overlap(&other.span)
    }

    fn union(&self, other: &Self) -> Option<Self> {
        // The speaker is a diarization label over one shared timeline, not a
        // coordinate space — overlapping spans always coalesce into one
        // redactable time span, like [`TimeSpan::union`]. The speaker is
        // carried only when both agree; a merged span across speakers honestly
        // carries none.
        //
        // [`TimeSpan::union`]: crate::primitive::TimeSpan::union
        let mut location = Self::new(self.span.union(&other.span));
        if self.speaker_id == other.speaker_id
            && let Some(speaker) = &self.speaker_id
        {
            location = location.with_speaker_id(speaker.clone());
        }
        Some(location)
    }

    fn span_cmp(&self, other: &Self) -> Ordering {
        // By duration: the longer utterance is the more specific match.
        self.span.duration_cmp(&other.span)
    }

    fn position_cmp(&self, other: &Self) -> Ordering {
        // Playback order: by start, then by end so a shorter span at the
        // same start sorts before a longer one.
        self.span.position_cmp(&other.span)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlaps_is_time_range_intersection() {
        let a = AudioLocation::from_millis(0, 1000);
        let b = AudioLocation::from_millis(500, 1500);
        assert!(a.overlaps(&b));
        // Touching but disjoint ranges do not overlap.
        let c = AudioLocation::from_millis(1000, 2000);
        assert!(!a.overlaps(&c));
    }

    #[test]
    fn overlaps_ignores_speaker() {
        let a = AudioLocation::from_millis(0, 1000).with_speaker_id("spk_0");
        let b = AudioLocation::from_millis(500, 1500).with_speaker_id("spk_1");
        // Different speakers talking over each other still overlap.
        assert!(a.overlaps(&b));
    }

    #[test]
    fn span_cmp_orders_by_duration() {
        let short = AudioLocation::from_millis(0, 200);
        let long = AudioLocation::from_millis(0, 1000);
        assert_eq!(short.span_cmp(&long), Ordering::Less);
    }

    #[test]
    fn position_cmp_is_playback_order() {
        let early = AudioLocation::from_millis(0, 5000);
        let late = AudioLocation::from_millis(1000, 2000);
        // Earlier start sorts first even though it is the longer span.
        assert_eq!(early.position_cmp(&late), Ordering::Less);
        // Same start: shorter end sorts first.
        let a = AudioLocation::from_millis(1000, 1500);
        let b = AudioLocation::from_millis(1000, 3000);
        assert_eq!(a.position_cmp(&b), Ordering::Less);
    }
}
