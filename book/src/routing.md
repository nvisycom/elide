# Language routing

A model trained for one language often beats a multilingual one on it: a
Polish NER model on Polish text, an English-only speech model on English
audio. `LanguageRouted` (feature `routing`) sends each request to the
backend registered for its language, and the rest to a fallback:

```rust
use elide::backend::BackendExt;
use elide::recognition::ner::NerRecognizer;
# use elide::recognition::ner::backend::MockBackend;

# fn main() -> elide::Result<()> {
# let (multilingual, polish, english) = (MockBackend::new(), MockBackend::new(), MockBackend::new());
let backend = multilingual
    .language_routed()
    .route("pl".parse()?, polish)
    .route("en".parse()?, english);

let ner = NerRecognizer::new(backend);
# Ok(())
# }
```

`language_routed` makes the backend it is called on the fallback;
`LanguageRouted::new(fallback)` is the same. A request goes to:

1. the route whose tag matches its language exactly, so `en-GB` goes to an
   `en-GB` route;
2. otherwise, the first route with the same primary language, so `en-GB`
   goes to an `en` route;
3. otherwise, and for requests with no language, the fallback.

The language is the call's primary language: the one in the
[scope](analyzers.md#the-scope), or one an enricher such as `lingua`
detected. NER, OCR, and speech-to-text requests all carry it.

A batch that mixes languages is split by route, each route gets one batch,
and the responses come back in the original order.

## Different labels per route

Models for different languages often support different labels. Scope each
route to its own [labels](ner.md#supported-labels):

```rust
use elide::backend::BackendExt;
use elide::prelude::*;
use elide::recognition::ner::backend::NerBackendExt;
# use elide::recognition::ner::backend::MockBackend;

# fn main() -> Result<()> {
# let (multilingual, polish) = (MockBackend::new(), MockBackend::new());
let backend = multilingual
    .label_scoped([builtins::PERSON_NAME.to_ref()])
    .language_routed()
    .route(
        "pl".parse()?,
        polish.label_scoped([builtins::PERSON_NAME.to_ref(), builtins::ADDRESS.to_ref()]),
    );
# Ok(())
# }
```

## Different backend types

All routes share one backend type. To route between different
implementations, such as a hosted model for one language and a local one
for another, wrap them in an enum that implements `Backend`.

## Usage per route

A router is not a provider: meter the backends it routes between, not the
router, and each call is charged to the model that served it.

```rust
use elide::backend::BackendExt;
use elide::backend::metrics::UsageCollector;
# use elide::recognition::ner::backend::MockBackend;

# fn main() -> elide::Result<()> {
# let (multilingual, polish) = (MockBackend::new(), MockBackend::new());
let usage = UsageCollector::new();

let backend = multilingual
    .metered(usage.clone())
    .language_routed()
    .route("pl".parse()?, polish.metered(usage.clone()));
# Ok(())
# }
```
