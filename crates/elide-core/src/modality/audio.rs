//! [`Audio`] modality: audio content addressed by time ranges.

use std::cmp::Ordering;

use bytes::Bytes;
use hipstr::HipStr;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::{Modality, ModalityData, ModalityLocation, ModalityReplacement};

/// Per-call payload a recognizer inspects for the [`Audio`] modality.
///
/// Carries the encoded audio bytes; an optional filename aids diagnostics
/// and encoding inference (the container format a decoder should expect).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct AudioData {
    /// Encoded audio bytes. Skipped by serde: the bytes are the raw payload,
    /// not metadata, and a serialized report has no need to carry the audio
    /// stream.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub bytes: Bytes,
    /// Original filename, when known.
    pub filename: Option<HipStr<'static>>,
}

impl AudioData {
    /// Wrap encoded audio bytes; filename unset.
    pub fn new(bytes: impl Into<Bytes>) -> Self {
        Self {
            bytes: bytes.into(),
            filename: None,
        }
    }

    /// Attach an original filename.
    #[must_use]
    pub fn with_filename(mut self, filename: impl Into<HipStr<'static>>) -> Self {
        self.filename = Some(filename.into());
        self
    }

    /// Lowercased extension derived from [`filename`](Self::filename),
    /// or `"wav"` when no filename is set or it has no extension.
    pub fn extension(&self) -> &str {
        self.filename
            .as_deref()
            .and_then(|name| name.rsplit_once('.'))
            .map(|(_, ext)| ext)
            .unwrap_or("wav")
    }
}

impl ModalityData for AudioData {}

/// Half-open `[start, end)` time range within audio content, in
/// milliseconds from the start of the stream.
///
/// Ordering and overlap consider only the time range. The optional
/// [`speaker_id`](Self::speaker_id) is a diarization label, not a
/// coordinate: two utterances from different speakers at the same instant
/// still overlap in time, so the speaker is carried for provenance but
/// excluded from comparison.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct AudioLocation {
    /// Milliseconds from the start of the stream where the range begins.
    pub start_ms: u64,
    /// Milliseconds from the start of the stream where the range ends
    /// (exclusive).
    pub end_ms: u64,
    /// Diarization label of the speaker, when a diarizer assigned one.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub speaker_id: Option<HipStr<'static>>,
}

impl AudioLocation {
    /// Location covering `start_ms..end_ms`, speaker unset.
    pub fn new(start_ms: u64, end_ms: u64) -> Self {
        Self {
            start_ms,
            end_ms,
            speaker_id: None,
        }
    }

    /// Attach a diarization speaker label.
    #[must_use]
    pub fn with_speaker_id(mut self, speaker_id: impl Into<HipStr<'static>>) -> Self {
        self.speaker_id = Some(speaker_id.into());
        self
    }

    /// Duration of the range in milliseconds (`end_ms - start_ms`).
    pub fn duration_ms(&self) -> u64 {
        self.end_ms.saturating_sub(self.start_ms)
    }

    /// Whether the range is empty (zero duration).
    pub fn is_empty(&self) -> bool {
        self.duration_ms() == 0
    }
}

impl ModalityLocation for AudioLocation {
    fn overlaps(&self, other: &Self) -> bool {
        // Time-range intersection; the speaker label is ignored, so two
        // speakers talking over each other still overlap.
        self.start_ms < other.end_ms && other.start_ms < self.end_ms
    }

    fn span_cmp(&self, other: &Self) -> Ordering {
        // By duration: the longer utterance is the more specific match.
        self.duration_ms().cmp(&other.duration_ms())
    }

    fn position_cmp(&self, other: &Self) -> Ordering {
        // Playback order: by start, then by end so a shorter span at the
        // same start sorts before a longer one.
        self.start_ms
            .cmp(&other.start_ms)
            .then(self.end_ms.cmp(&other.end_ms))
    }
}

/// What an audio operator produces to hide an entity: an acoustic
/// treatment applied to the entity's time range.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum AudioReplacement {
    /// Replace the range with silence, preserving its duration so the
    /// timeline does not shift.
    Silenced,
    /// Cut the range out entirely, shortening the stream.
    Removed,
}

impl ModalityReplacement for AudioReplacement {}

/// Audio modality: data is [`AudioData`], locations are
/// [`AudioLocation`] time ranges, replacements are [`AudioReplacement`].
#[derive(Debug, Clone, Copy)]
pub struct Audio;

impl Modality for Audio {
    type Data = AudioData;
    type Location = AudioLocation;
    type Replacement = AudioReplacement;

    const NAME: &'static str = "audio";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlaps_is_time_range_intersection() {
        let a = AudioLocation::new(0, 1000);
        let b = AudioLocation::new(500, 1500);
        assert!(a.overlaps(&b));
        // Touching but disjoint ranges do not overlap.
        let c = AudioLocation::new(1000, 2000);
        assert!(!a.overlaps(&c));
    }

    #[test]
    fn overlaps_ignores_speaker() {
        let a = AudioLocation::new(0, 1000).with_speaker_id("spk_0");
        let b = AudioLocation::new(500, 1500).with_speaker_id("spk_1");
        // Different speakers talking over each other still overlap.
        assert!(a.overlaps(&b));
    }

    #[test]
    fn span_cmp_orders_by_duration() {
        let short = AudioLocation::new(0, 200);
        let long = AudioLocation::new(0, 1000);
        assert_eq!(short.span_cmp(&long), Ordering::Less);
    }

    #[test]
    fn position_cmp_is_playback_order() {
        let early = AudioLocation::new(0, 5000);
        let late = AudioLocation::new(1000, 2000);
        // Earlier start sorts first even though it is the longer span.
        assert_eq!(early.position_cmp(&late), Ordering::Less);
        // Same start: shorter end sorts first.
        let a = AudioLocation::new(1000, 1500);
        let b = AudioLocation::new(1000, 3000);
        assert_eq!(a.position_cmp(&b), Ordering::Less);
    }

    #[test]
    fn extension_falls_back_to_wav() {
        let d = AudioData::new(Bytes::new());
        assert_eq!(d.extension(), "wav");
        let named = d.with_filename("call.MP3");
        assert_eq!(named.extension(), "MP3");
    }
}
