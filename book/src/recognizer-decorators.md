# Recognizer decorators

A decorator wraps a recognizer and is a recognizer itself, so decorators
compose with each other and go anywhere a recognizer does. Each changes how
the wrapped recognizer runs, and with it what comes out: which errors fail
the call, which findings are kept, and how confident they are, as each
section below says. They live in `elide::recognition::decorator`.

`RecognizerExt` applies each by method, so a stack reads in the order it
wraps, innermost first, as `BackendExt` does for backends:

```rust
use elide::prelude::*;
use elide::recognition::decorator::RecognizerExt;
use elide::recognition::decorator::calibrated::{Calibration, Multiplier};
use elide::recognition::decorator::chunked::CharWindows;
use elide::recognition::pattern::PatternRecognizer;

# fn main() -> Result<()> {
const SCALE: Multiplier = Multiplier::new(0.8).unwrap();

let recognizer = PatternRecognizer::builder()
    .with_builtin_patterns()
    .build()?
    .chunked_with(CharWindows::for_gliner())
    .calibrated(Calibration::scale(SCALE))
    .resilient();
# Ok(())
# }
```

## Resilience

An analysis fails if any recognizer fails, because a recognizer that errored
has not vouched that the content is clean. Where some coverage is better
than none, such as a hosted model that may time out, opt out per
recognizer:

```rust
use elide::recognition::decorator::RecognizerExt;
use elide::recognition::ner::NerRecognizer;

# fn main() {
let ner = NerRecognizer::mock().resilient();
# }
```

A `Resilient` logs the inner recognizer's errors and returns no
entities in their place.

## Chunking

Many models accept only so much input at once. `Chunked` splits a
long text into overlapping windows, runs the recognizer on each, and moves
the findings back to their place in the full text:

```rust
use elide::recognition::decorator::RecognizerExt;
use elide::recognition::decorator::chunked::CharWindows;
use elide::recognition::ner::NerRecognizer;

# fn main() {
// 384-word windows that overlap by 64 words.
let ner = NerRecognizer::mock().chunked();

// Windows sized for GLiNER's 512-token limit.
let ner = NerRecognizer::mock().chunked_with(CharWindows::for_gliner());
# }
```

A finding seen twice where windows overlap is kept once. `WordWindows` and
`CharWindows` take a custom size and overlap, and the `Chunking` trait lets
you write your own.

## Escalation

`Escalating` always runs a cheap recognizer, and runs an expensive one only
where a `Decider` says so. `WeakBaseline` escalates when the cheap
recognizer's findings are not confident enough:

```rust
use elide::prelude::*;
use elide::recognition::decorator::RecognizerExt;
use elide::recognition::decorator::escalating::WeakBaseline;
use elide::recognition::llm::LlmRecognizer;
use elide::recognition::llm::backend::MockBackend;
use elide::recognition::pattern::PatternRecognizer;

# fn main() -> Result<()> {
let patterns = PatternRecognizer::builder().with_builtin_patterns().build()?;
let llm = LlmRecognizer::<MockBackend<Text>>::mock();

let recognizer = patterns.escalating(llm, WeakBaseline::new(ConfidenceThreshold::BASELINE));
# Ok(())
# }
```

## Calibration

Recognizers score on different scales: a pattern hit is often `1.0`, a NER
model's scores spread across `0.3–0.9`. `Calibrated` scales a
recognizer's confidence by a `Calibration`, a multiplier for every label and,
where a model runs hot or cold on some labels, one per label, so its findings
compare fairly with the others' when overlaps are fused and arbitrated. Each
rescaled finding records the multiplier on its audit trail.

A `Multiplier` is checked as it is made (finite and non-negative), so building
a `Calibration` cannot fail; `Multiplier::new` is a `const fn`, so a literal
one is checked when the program compiles. A label is named by its `LabelRef`, from
`builtins` or your catalog, never a bare string: label ids are lowercase
(`phone_number`), and a misspelt one would never match.

```rust
use elide::entity::builtins;
use elide::prelude::*;
use elide::recognition::decorator::RecognizerExt;
use elide::recognition::decorator::calibrated::{Calibration, Multiplier};
use elide::recognition::pattern::PatternRecognizer;

# fn main() -> Result<()> {
let patterns = PatternRecognizer::builder().with_builtin_patterns().build()?;

const PATTERNS: Multiplier = Multiplier::new(0.8).unwrap();
const PHONES: Multiplier = Multiplier::new(0.6).unwrap();

let recognizer = patterns
    .calibrated(Calibration::scale(PATTERNS).with_label(builtins::PHONE_NUMBER.to_ref(), PHONES));
# Ok(())
# }
```
