//! [`Transcription`]: timestamped speech-to-text output addressable as text.
//!
//! A transcription is what makes audio *recognizable*: a recognizer reads
//! its [`text`] like any other string, finds a match
//! at a byte range, and [`resolve`]s that range back
//! to the [`TimeSpan`] of the audio it was spoken in — via the per-word
//! timings the transcription carries. The same shape serves any medium that
//! turns a stream into timestamped text.
//!
//! [`text`]: Transcription::text
//! [`resolve`]: Transcription::resolve

use std::ops::Range;

use hipstr::HipStr;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::AudioLocation;
use crate::primitive::{Confidence, LanguageTag, TimeSpan};

/// Separator inserted between segments when building the flat transcript
/// text, so adjacent segments don't run their words together.
const SEGMENT_SEPARATOR: &str = " ";

/// Timestamped transcript of an audio stream.
///
/// An ordered set of [`TranscriptSegment`]s. The flat
/// [`text`] — the segments joined — is what a recognizer
/// inspects; [`resolve`] maps a byte range of that text back
/// to the [`TimeSpan`] it occupies, using the segments' (and their words')
/// timings. Empty when the backend produced nothing (silence, or a no-op
/// backend).
///
/// [`text`]: Self::text
/// [`resolve`]: Self::resolve
#[derive(Debug, Clone, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Transcription {
    /// Segments in stream order.
    segments: Vec<TranscriptSegment>,
    /// The segments' text joined by [`SEGMENT_SEPARATOR`], cached so
    /// recognition and byte-range resolution share one flat string. Each
    /// segment's text begins at a known offset within it (see
    /// [`segment_offsets`]).
    ///
    /// [`segment_offsets`]: Self::segment_offsets
    text: String,
}

/// One segment of a [`Transcription`]: a span of audio and the text
/// recognised within it, with optional diarization, language, confidence,
/// and per-word timings.
///
/// `speaker_id` is populated only by backends with diarization;
/// `language` by backends that emit per-segment language detection;
/// `confidence` when the backend reports one; `words` when it emits a
/// word-level breakdown (which is what lets a sub-segment range resolve to
/// a tighter span than the whole segment).
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TranscriptSegment {
    /// Time span the segment covers within the stream.
    pub span: TimeSpan,
    /// Recognised text for this segment.
    pub text: String,
    /// Diarization speaker label, when the backend assigned one.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub speaker_id: Option<HipStr<'static>>,
    /// Detected language for this segment, when the backend reported one.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub language: Option<LanguageTag>,
    /// Backend confidence in the segment, when reported.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub confidence: Option<Confidence>,
    /// Per-word timings within the segment, when the backend emitted them.
    /// Empty otherwise; resolution then falls back to the segment span.
    #[cfg_attr(feature = "serde", serde(default))]
    pub words: Vec<TranscriptWord>,
}

impl TranscriptSegment {
    /// A segment covering `span` with the given text and no optional fields
    /// set.
    pub fn new(span: TimeSpan, text: impl Into<String>) -> Self {
        Self {
            span,
            text: text.into(),
            speaker_id: None,
            language: None,
            confidence: None,
            words: Vec::new(),
        }
    }

    /// Attach a diarization speaker label.
    #[must_use]
    pub fn with_speaker_id(mut self, speaker_id: impl Into<HipStr<'static>>) -> Self {
        self.speaker_id = Some(speaker_id.into());
        self
    }

    /// Attach a per-segment detected language.
    #[must_use]
    pub fn with_language(mut self, language: LanguageTag) -> Self {
        self.language = Some(language);
        self
    }

    /// Attach a segment-level confidence.
    #[must_use]
    pub fn with_confidence(mut self, confidence: Confidence) -> Self {
        self.confidence = Some(confidence);
        self
    }

    /// Attach a word-level breakdown.
    #[must_use]
    pub fn with_words(mut self, words: Vec<TranscriptWord>) -> Self {
        self.words = words;
        self
    }
}

/// One word within a [`TranscriptSegment`], with its own time span.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TranscriptWord {
    /// Time span the word covers within the stream.
    pub span: TimeSpan,
    /// The word text, as it appears in the segment text.
    pub text: String,
    /// Per-word confidence, when reported.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub confidence: Option<Confidence>,
}

impl TranscriptWord {
    /// A word covering `span` with the given text and no confidence set.
    pub fn new(span: TimeSpan, text: impl Into<String>) -> Self {
        Self {
            span,
            text: text.into(),
            confidence: None,
        }
    }

    /// Attach a per-word confidence.
    #[must_use]
    pub fn with_confidence(mut self, confidence: Confidence) -> Self {
        self.confidence = Some(confidence);
        self
    }
}

impl Transcription {
    /// Build a transcription from segments, computing the flat text.
    #[must_use]
    pub fn new(segments: Vec<TranscriptSegment>) -> Self {
        let text = segments
            .iter()
            .map(|s| s.text.as_str())
            .collect::<Vec<_>>()
            .join(SEGMENT_SEPARATOR);
        Self { segments, text }
    }

    /// The flat transcript text a recognizer inspects: the segments' text
    /// joined.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Segments in stream order.
    #[must_use]
    pub fn segments(&self) -> &[TranscriptSegment] {
        &self.segments
    }

    /// Whether the transcription has no segments.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    /// Byte offset where each segment's text begins within [`text`].
    ///
    /// Mirrors how `text` is built: segment `i` starts after the previous
    /// segments plus one separator each.
    ///
    /// [`text`]: Self::text
    fn segment_offsets(&self) -> impl Iterator<Item = (usize, &TranscriptSegment)> {
        let mut offset = 0;
        self.segments.iter().map(move |segment| {
            let start = offset;
            offset += segment.text.len() + SEGMENT_SEPARATOR.len();
            (start, segment)
        })
    }

    /// Resolve a byte `range` of [`text`] to the [`AudioLocation`] it was
    /// spoken in: the time span plus the speaker, when diarization assigned
    /// one *and* the range stays within a single speaker's segments.
    ///
    /// The span runs from the start of the first overlapped word (or segment,
    /// when word timings are absent) to the end of the last, falling back to
    /// whole-segment spans without per-word timings. The speaker is carried
    /// through only when every overlapped segment shares it — a range that
    /// crosses a speaker change is left speaker-less rather than mis-attributed.
    /// `None` when the range overlaps no segment (out of bounds, or an empty
    /// transcription) — the caller drops such a match.
    ///
    /// [`text`]: Self::text
    #[must_use]
    pub fn resolve(&self, range: Range<usize>) -> Option<AudioLocation> {
        let mut start_us: Option<u64> = None;
        let mut end_us: Option<u64> = None;
        // `Some(None)` = no speaker seen yet; `Some(Some(id))` = one consistent
        // speaker; `None` = conflicting speakers, so attribute to nobody.
        let mut speaker: Option<Option<&HipStr<'static>>> = Some(None);

        for (seg_start, segment) in self.segment_offsets() {
            let seg_end = seg_start + segment.text.len();
            // Skip segments the range does not touch (half-open overlap).
            if range.start >= seg_end || range.end <= seg_start {
                continue;
            }

            // Intersect the range with this segment, in segment-local bytes.
            let local_start = range.start.saturating_sub(seg_start);
            let local_end = (range.end.min(seg_end)).saturating_sub(seg_start);

            let segment_span = if segment.words.is_empty() {
                segment.span
            } else {
                word_span(segment, local_start..local_end).unwrap_or(segment.span)
            };

            start_us = Some(start_us.map_or(segment_span.start_micros(), |s| {
                s.min(segment_span.start_micros())
            }));
            end_us = Some(end_us.map_or(segment_span.end_micros(), |e| {
                e.max(segment_span.end_micros())
            }));

            // Track the speaker across overlapped segments: keep it only while
            // every segment agrees, otherwise collapse to "no speaker".
            speaker = match (speaker, segment.speaker_id.as_ref()) {
                (Some(None), seen) => Some(seen),
                (Some(Some(prev)), Some(seen)) if prev == seen => Some(Some(prev)),
                (Some(Some(_)), _) | (None, _) => None,
            };
        }

        let span = TimeSpan::new(start_us?, end_us?);
        let mut location = AudioLocation::new(span);
        if let Some(Some(id)) = speaker {
            location = location.with_speaker_id(id.clone());
        }
        Some(location)
    }
}

/// Span covering the words of `segment` that overlap the segment-local byte
/// `range`, by walking each word's byte extent within the segment text.
///
/// `None` when no word overlaps (the caller then falls back to the segment
/// span).
fn word_span(segment: &TranscriptSegment, range: Range<usize>) -> Option<TimeSpan> {
    let mut start_us: Option<u64> = None;
    let mut end_us: Option<u64> = None;
    let mut search_from = 0;

    for word in &segment.words {
        // Locate the word in the segment text from where the last word
        // ended, so repeated words resolve to successive occurrences.
        let Some(rel) = segment.text[search_from..].find(word.text.as_str()) else {
            continue;
        };
        let word_start = search_from + rel;
        let word_end = word_start + word.text.len();
        search_from = word_end;

        // Half-open overlap with the requested range.
        if range.start >= word_end || range.end <= word_start {
            continue;
        }
        start_us = Some(start_us.map_or(word.span.start_micros(), |s| {
            s.min(word.span.start_micros())
        }));
        end_us = Some(end_us.map_or(word.span.end_micros(), |e| e.max(word.span.end_micros())));
    }

    Some(TimeSpan::new(start_us?, end_us?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(start_ms: u64, end_ms: u64, text: &str) -> TranscriptWord {
        TranscriptWord::new(TimeSpan::from_millis(start_ms, end_ms), text)
    }

    /// "Call Alice at 555-1234" as one segment with per-word timings.
    fn phone_segment() -> TranscriptSegment {
        TranscriptSegment::new(TimeSpan::from_millis(0, 1_800), "Call Alice at 555-1234")
            .with_words(vec![
                word(0, 400, "Call"),
                word(400, 900, "Alice"),
                word(900, 1_100, "at"),
                word(1_100, 1_800, "555-1234"),
            ])
    }

    #[test]
    fn text_is_segments_joined() {
        let t = Transcription::new(vec![
            TranscriptSegment::new(TimeSpan::from_millis(0, 500), "hello"),
            TranscriptSegment::new(TimeSpan::from_millis(500, 1_000), "world"),
        ]);
        assert_eq!(t.text(), "hello world");
    }

    #[test]
    fn resolve_maps_a_word_range_to_its_timing() {
        let t = Transcription::new(vec![phone_segment()]);
        // "555-1234" is at bytes 14..22.
        let loc = t.resolve(14..22).expect("in bounds");
        assert_eq!(loc.span.start_millis(), 1_100);
        assert_eq!(loc.span.end_millis(), 1_800);
    }

    #[test]
    fn resolve_spans_multiple_words() {
        let t = Transcription::new(vec![phone_segment()]);
        // "Alice at 555-1234" -> bytes 5..22 -> Alice start to phone end.
        let loc = t.resolve(5..22).expect("in bounds");
        assert_eq!(loc.span.start_millis(), 400);
        assert_eq!(loc.span.end_millis(), 1_800);
    }

    #[test]
    fn resolve_falls_back_to_segment_span_without_words() {
        let t = Transcription::new(vec![TranscriptSegment::new(
            TimeSpan::from_millis(200, 900),
            "no word timings here",
        )]);
        let loc = t.resolve(3..7).expect("in bounds");
        assert_eq!(loc.span.start_millis(), 200);
        assert_eq!(loc.span.end_millis(), 900);
    }

    #[test]
    fn resolve_crosses_segment_boundary() {
        let t = Transcription::new(vec![
            TranscriptSegment::new(TimeSpan::from_millis(0, 500), "alice"),
            TranscriptSegment::new(TimeSpan::from_millis(600, 1_000), "bob"),
        ]);
        // "alice bob" -> bytes 0..9 spans both segments.
        let loc = t.resolve(0..9).expect("in bounds");
        assert_eq!(loc.span.start_millis(), 0);
        assert_eq!(loc.span.end_millis(), 1_000);
    }

    #[test]
    fn resolve_carries_a_single_speaker() {
        let t = Transcription::new(vec![
            TranscriptSegment::new(TimeSpan::from_millis(0, 500), "alice").with_speaker_id("spk_0"),
        ]);
        let loc = t.resolve(0..5).expect("in bounds");
        assert_eq!(loc.speaker_id.as_deref(), Some("spk_0"));
    }

    #[test]
    fn resolve_drops_speaker_across_a_speaker_change() {
        let t = Transcription::new(vec![
            TranscriptSegment::new(TimeSpan::from_millis(0, 500), "alice").with_speaker_id("spk_0"),
            TranscriptSegment::new(TimeSpan::from_millis(600, 1_000), "bob")
                .with_speaker_id("spk_1"),
        ]);
        // bytes 0..9 cross both speakers -> attributed to neither.
        let loc = t.resolve(0..9).expect("in bounds");
        assert!(loc.speaker_id.is_none());
    }

    #[test]
    fn resolve_out_of_bounds_is_none() {
        let t = Transcription::new(vec![phone_segment()]);
        assert!(t.resolve(100..200).is_none());
    }

    #[test]
    fn resolve_on_empty_transcription_is_none() {
        let t = Transcription::default();
        assert!(t.resolve(0..5).is_none());
    }
}
