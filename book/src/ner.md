# Named-entity models

Names, addresses, and organizations have no fixed shape, so patterns cannot
find them. `NerRecognizer` (feature `ner`) asks a named-entity model instead.
The model sits behind a [backend](backends.md); the recognizer turns its
answers into entities.

```rust
use elide::prelude::*;
use elide::recognition::ner::NerRecognizer;
# use elide::recognition::ner::backend::MockBackend;

# fn main() {
# let backend = MockBackend::new();
let ner = NerRecognizer::new(backend).with_name("gliner");

let analyzer = Analyzer::<Text>::new().with_recognizer(ner);
# }
```

On each call, the recognizer asks the model for every label in the
[scope's catalog](analyzers.md#the-scope). A zero-shot model such as GLiNER
gets each label with its description, and finds those. A model with a fixed
label set ignores the list, and its backend maps the model's labels to
Elide's.

## Supported labels

Few models can find every label in the catalog, and a zero-shot model is
slower and less accurate the more labels it is asked for. Declare what the
model supports by scoping its backend to them:

```rust
use elide::prelude::*;
use elide::recognition::ner::NerRecognizer;
use elide::recognition::ner::backend::NerBackendExt;
# use elide::recognition::ner::backend::MockBackend;

# fn main() {
# let gliner = MockBackend::new();
let backend = gliner.label_scoped([
    builtins::PERSON_NAME.to_ref(),
    builtins::ADDRESS.to_ref(),
    builtins::ORGANIZATION_NAME.to_ref(),
]);

let ner = NerRecognizer::new(backend);
# }
```

The model is then asked only for the catalog labels it supports, and any
other label it returns is dropped. When none of the requested labels is
supported, the model is not called at all.

The supported labels belong to the model, so they sit on the backend, not
the recognizer. Behind a [language router](routing.md), each route can
declare its own.

Three decorators in `elide::recognition::ner::backend` shape a model's labels
and scores, each also a method of `NerBackendExt`:

| Decorator | Method | Does |
|---|---|---|
| `LabelScoped` | `label_scoped(labels)` | Asks the model only for these labels, and keeps only these |
| `LabelExcluded` | `label_excluded(labels)` | Drops spans with these labels, such as a generic `MISC` |
| `ScoreScaled` | `score_scaled(labels)` | Boosts these labels' scores by 1.2×, or by `with_multiplier(m)` |

Elide ships no NER model of its own: you bring one, hosted or local, by
writing a small backend for it. See [Custom backends](custom-backends.md).
The `mocks` feature provides a `MockBackend` that finds nothing, for tests.

## Long texts

Many NER models accept only a few hundred tokens. Wrap the recognizer in a
`ChunkedRecognizer` to split long texts into overlapping windows; see
[Recognizer decorators](recognizer-decorators.md).

## Text in images and audio

A NER recognizer works on any modality that has text: on an image, it reads
the text OCR found; on audio, the transcript. See
[Images and OCR](images.md) and [Audio and speech](audio.md).
