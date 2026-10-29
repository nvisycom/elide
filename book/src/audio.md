# Audio and speech

A recording discloses what was said: a caller reading out a card number, a
patient giving a date of birth. An `SttEnricher` (feature `stt`) transcribes
the audio before the recognizers run. Every text recognizer then works on
the transcript, and its findings are mapped back to the time interval in
which the words were spoken:

```rust
use elide::enrichment::stt::SttEnricher;
use elide::prelude::operators::*;
use elide::prelude::*;
use elide::recognition::pattern::PatternRecognizer;

# fn main() -> Result<()> {
# let speech_to_text = elide::enrichment::stt::MockBackend::new();
let analyzer = Analyzer::<Audio>::new()
    .with_enricher(SttEnricher::new(speech_to_text))
    .with_recognizer(PatternRecognizer::builder().with_builtin_patterns().build()?);

let anonymizer = Anonymizer::<Audio>::new()
    .with(Rule::label(builtins::PAYMENT_CARD.to_ref(), Beep::default()))
    .with(Rule::fallback(Silence));
# Ok(())
# }
```

`Silence` replaces the interval with silence, and `Beep` with a tone, so a
listener can tell something was removed.

The speech-to-text model is a [backend](backends.md) of your own: it takes an
`SttRequest` (the audio, its format, and a language hint) and returns the
transcript as `TranscriptSegment`s with their time spans. See
[Custom backends](custom-backends.md).

WAV files are read with `codec-wav`, and MP3 files with `codec-mp3`.
