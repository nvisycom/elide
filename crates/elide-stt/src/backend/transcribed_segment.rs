//! [`TranscribedSegment`] and [`TranscribedWord`]: the per-segment unit a
//! [`SttBackend`] produces.
//!
//! A segment pairs the recognised text with the [`TimeSpan`] it occupies
//! in the source clip, plus optional diarization and per-word timings. The
//! span shares the coordinate space of [`AudioLocation`], so a range
//! detected in the segment text lifts straight back to audio time.
//!
//! [`SttBackend`]: super::SttBackend
//! [`AudioLocation`]: elide_core::modality::audio::AudioLocation

use elide_core::primitive::{Confidence, LanguageTag, TimeSpan};
use hipstr::HipStr;

/// One transcription segment from an [`SttBackend`].
///
/// Carries the [`TimeSpan`] it covers, the recognised text, and a handful
/// of optional fields providers may or may not fill in. `speaker_id` is
/// populated only by providers with diarization (Deepgram, AssemblyAI, …)
/// — single-speaker transcribers leave it `None`. `language` is populated
/// by providers that emit per-segment language detection, handy for
/// code-switching audio. `words` carries a word-level breakdown when the
/// provider emitted one, which is what lets a sub-segment text range
/// resolve to a tighter span than the whole segment.
///
/// [`SttBackend`]: super::SttBackend
#[derive(Debug, Clone, PartialEq)]
pub struct TranscribedSegment {
    /// Time span the segment covers within the source clip.
    pub span: TimeSpan,
    /// Recognised text for this segment.
    pub text: String,
    /// Speaker label, when the backend performed diarization.
    pub speaker_id: Option<HipStr<'static>>,
    /// Detected language for this segment, when the backend reported one.
    pub language: Option<LanguageTag>,
    /// Backend confidence in the segment, when reported.
    pub confidence: Option<Confidence>,
    /// Word-level breakdown, when the backend emitted one. Empty otherwise.
    pub words: Vec<TranscribedWord>,
}

impl TranscribedSegment {
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
    pub fn with_words(mut self, words: Vec<TranscribedWord>) -> Self {
        self.words = words;
        self
    }
}

/// One word inside a [`TranscribedSegment`].
///
/// Populated by backends that emit word-level timestamps (OpenAI Whisper
/// with `timestamp_granularities=["word"]`, Deepgram, …). The word's
/// [`TimeSpan`] is what a lift uses to map a byte range of the segment text
/// onto a tighter slice of audio than the whole segment.
#[derive(Debug, Clone, PartialEq)]
pub struct TranscribedWord {
    /// Time span the word covers within the source clip.
    pub span: TimeSpan,
    /// The word as the backend transcribed it.
    pub text: String,
    /// Per-word confidence, when reported.
    pub confidence: Option<Confidence>,
}

impl TranscribedWord {
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
