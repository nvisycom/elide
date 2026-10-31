# Recognizer decorators

A decorator wraps a recognizer and is a recognizer itself, so decorators
compose with each other and go anywhere a recognizer does. Each changes how
the wrapped recognizer runs, and with it what comes out: which errors fail
the call, which findings are kept, and how confident they are, as each
section below says. They live in `elide::recognition::decorator` and
`elide::recognition::escalation`.

## Resilience

An analysis fails if any recognizer fails, because a recognizer that errored
has not vouched that the content is clean. Where some coverage is better
than none, such as a hosted model that may time out, opt out per
recognizer:

```rust
use elide::recognition::decorator::ResilientRecognizer;
use elide::recognition::ner::NerRecognizer;

# fn main() {
let ner = ResilientRecognizer::new(NerRecognizer::mock());
# }
```

A `ResilientRecognizer` logs the inner recognizer's errors and returns no
entities in their place.

## Chunking

Many models accept only so much input at once. `ChunkedRecognizer` splits a
long text into overlapping windows, runs the recognizer on each, and moves
the findings back to their place in the full text:

```rust
use elide::recognition::decorator::{CharWindows, ChunkedRecognizer};
use elide::recognition::ner::NerRecognizer;

# fn main() {
// 384-word windows that overlap by 64 words.
let ner = ChunkedRecognizer::new(NerRecognizer::mock());

// Windows sized for GLiNER's 512-token limit.
let ner = ChunkedRecognizer::with_chunking(NerRecognizer::mock(), CharWindows::for_gliner());
# }
```

A finding seen twice where windows overlap is kept once. `WordWindows` and
`CharWindows` take a custom size and overlap, and the `Chunking` trait lets
you write your own.

## Escalation

`EscalatingRecognizer` always runs a cheap recognizer, and runs an expensive
one only where a `Decider` says so. `ConfidenceDecider` escalates when the
cheap recognizer's findings are not confident enough:

```rust
use elide::prelude::*;
use elide::recognition::escalation::{ConfidenceDecider, EscalatingRecognizer};
use elide::recognition::llm::LlmRecognizer;
use elide::recognition::llm::backend::MockBackend;
use elide::recognition::pattern::PatternRecognizer;

# fn main() -> Result<()> {
let patterns = PatternRecognizer::builder().with_builtin_patterns().build()?;
let llm = LlmRecognizer::<MockBackend<Text>>::mock();

let recognizer = EscalatingRecognizer::new(
    patterns,
    llm,
    ConfidenceDecider::new(ConfidenceThreshold::BASELINE),
);
# Ok(())
# }
```
