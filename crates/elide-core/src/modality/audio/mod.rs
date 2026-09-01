//! [`Audio`] modality: audio content addressed by time ranges.

mod data;
mod location;
mod replacement;
mod transcription;

use std::ops::Range;

pub use self::data::AudioData;
pub use self::location::AudioLocation;
pub use self::replacement::{AudioReplacement, Waveform};
pub use self::transcription::{TranscriptSegment, TranscriptWord, Transcription};
use super::{Modality, ModalityArtifact, TextRecognizable};

/// Audio modality: data is [`AudioData`], locations are
/// [`AudioLocation`] time ranges, replacements are [`AudioReplacement`].
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Audio;

impl Modality for Audio {
    type Artifact = Transcription;
    type Data = AudioData;
    type Location = AudioLocation;
    type Replacement = AudioReplacement;

    const NAME: &'static str = "audio";
}

impl ModalityArtifact for Transcription {}

impl TextRecognizable for Audio {
    /// The transcript text a recognizer inspects: the [`Transcription`] an
    /// enricher stamped onto the call, or `""` when it is empty (a clip that
    /// was never transcribed) — a recognizer then finds nothing, rather than
    /// erroring.
    fn as_text<'a>(_data: &'a AudioData, artifact: &'a Transcription) -> &'a str {
        artifact.text()
    }

    /// Resolve a transcript byte `range` to the audio time it was spoken in.
    ///
    /// Unlike the byte-based text modalities, audio's location is a time
    /// span, so `locate` resolves `range` immediately against the
    /// [`Transcription`]'s word timings rather than deferring to a lift.
    /// Returns `None` when the range resolves to nothing (an empty transcript,
    /// or out of bounds) — there is no time span to address, so the caller
    /// drops the match.
    fn locate(
        range: Range<usize>,
        _data: &AudioData,
        artifact: &Transcription,
    ) -> Option<AudioLocation> {
        // An empty transcript, or a range no segment covers: nothing to
        // address. `resolve` yields the time span *and* the speaker (when
        // diarized).
        artifact.resolve(range)
    }
}

#[cfg(test)]
mod tests {
    use super::{TranscriptSegment, TranscriptWord, *};
    use crate::primitive::TimeSpan;
    use crate::recognition::{RecognizerContext, Scope};

    #[test]
    fn as_text_is_empty_without_a_transcript() {
        let data = AudioData::new(bytes::Bytes::new());
        let scope = Scope::new();
        let ctx = RecognizerContext::<Audio>::new(&scope);
        assert_eq!(Audio::as_text(&data, &ctx.artifact), "");
    }

    /// A context whose artifacts carry the phone-number transcript.
    fn phone_context(scope: &Scope) -> RecognizerContext<'_, Audio> {
        let segment =
            TranscriptSegment::new(TimeSpan::from_millis(0, 1_800), "Call Alice at 555-1234")
                .with_words(vec![TranscriptWord::new(
                    TimeSpan::from_millis(1_100, 1_800),
                    "555-1234",
                )]);
        let mut ctx = RecognizerContext::new(scope);
        ctx.artifact = Transcription::new(vec![segment]);
        ctx
    }

    #[test]
    fn as_text_reads_the_transcript_artifact() {
        let data = AudioData::new(bytes::Bytes::new());
        let scope = Scope::new();
        let ctx = phone_context(&scope);
        assert_eq!(
            Audio::as_text(&data, &ctx.artifact),
            "Call Alice at 555-1234"
        );
    }

    #[test]
    fn locate_resolves_a_transcript_range_to_audio_time() {
        let data = AudioData::new(bytes::Bytes::new());
        let scope = Scope::new();
        let ctx = phone_context(&scope);
        // "555-1234" is at bytes 14..22.
        let loc = Audio::locate(14..22, &data, &ctx.artifact).expect("range resolves");
        assert_eq!(loc.span.start_millis(), 1_100);
        assert_eq!(loc.span.end_millis(), 1_800);
    }

    #[test]
    fn locate_without_transcript_is_none() {
        let data = AudioData::new(bytes::Bytes::new());
        let scope = Scope::new();
        let ctx = RecognizerContext::<Audio>::new(&scope);
        // No transcript: the range can't be placed, so no location.
        assert!(Audio::locate(0..5, &data, &ctx.artifact).is_none());
    }
}
