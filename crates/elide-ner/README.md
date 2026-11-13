# elide-ner

[![Build](https://img.shields.io/github/actions/workflow/status/nvisycom/elide/build.yml?branch=main&label=build%20%26%20test&style=flat-square)](https://github.com/nvisycom/elide/actions/workflows/build.yml)

Named-entity recognition for PII/PHI detection.

## Overview

Some sensitive data has no fixed shape: a person's name, an organization, a
place. Recognizing it takes a model that reads the surrounding language rather
than a regular expression. This crate provides the recognizer that turns a
model's spans into located entities, over a pluggable backend, so the model
runs wherever suits the deployment: in process or as a hosted service. Elide
ships no NER model; a backend is a small adapter you write for yours.

A backend returns spans already carrying the toolkit's canonical labels, so
consumers reason about one taxonomy whatever the upstream model. Which labels
a model supports, which it emits but should be dropped, and how its scores
compare are properties of the model, so they are decorators on its backend
rather than recognizer settings.

```rust
use elide_core::entity::{LabelCatalog, LabelRef, builtins};
use elide_core::modality::text::{Text, TextData};
use elide_core::recognition::{Context, Recognizer, Scope, Subject};
use elide_ner::NerRecognizer;
use elide_ner::backend::NerBackendExt;
# use elide_core::backend::Backend;
# use elide_core::entity::audit::ModelId;
# use elide_ner::backend::{NerRequest, NerResponse, NerSpan};
#
# /// Stands in for a real model: finds a person name at the start.
# struct MyModel;
#
# #[async_trait::async_trait]
# impl Backend for MyModel {
#     type Request<'a> = NerRequest<'a>;
#     type Response = NerResponse;
#
#     fn provenance(&self) -> ModelId {
#         ModelId { name: "my-model".into(), ..ModelId::default() }
#     }
#
#     async fn call(&self, _request: NerRequest<'_>) -> elide_core::Result<NerResponse> {
#         Ok(NerResponse::new(vec![NerSpan::new("person_name", 0.8, 0..8)]))
#     }
# }

# #[tokio::main(flavor = "current_thread")]
# async fn main() -> elide_core::Result<()> {
let backend = MyModel
    .label_scoped([builtins::PERSON_NAME.to_ref(), builtins::ADDRESS.to_ref()])
    .label_excluded([LabelRef::new("misc")])
    .score_scaled([builtins::PERSON_NAME.to_ref()], 1.2);
let recognizer = NerRecognizer::new(backend);

let scope = Scope::new().with_catalog(LabelCatalog::with_builtins());
let subject = Subject::<Text>::new(TextData::new("Jane Doe lives in Berlin"));
let entities = recognizer.recognize(&subject, &Context::new(&scope)).await?;
assert_eq!(entities[0].label.as_str(), "person_name");
# Ok(())
# }
```

The crate holds:

- **`NerRecognizer`** — the recognizer. Per call it asks its backend for the
  catalog's labels, sorted by id, and turns every span into an entity whose
  audit event names the backend's model. `recognize_batch` sends one batch
  for many subjects.
- **`backend::NerRequest`** and **`backend::NerResponse`** — the contract a
  NER backend speaks: text, the labels to look for (`None` for a fixed-label
  model), and the language in; canonical `NerSpan`s out.
- **`backend::LabelScoped`**, **`LabelExcluded`**, and **`ScoreScaled`** —
  decorators that scope a model to the labels it supports, drop labels it
  emits but should not, and rescale chosen labels' scores. `NerBackendExt`
  applies each by method.
- **`backend::MockBackend`** (feature `mocks`) — a backend that finds
  nothing, for tests and examples.

## Documentation

See [`docs/`](../../docs/) for architecture, security, and API documentation.

## Changelog

See [CHANGELOG.md](../../CHANGELOG.md) for release notes and version history.

## License

Apache 2.0 License, see [LICENSE](../../LICENSE)

## Support

- **Documentation**: [docs.nvisy.com](https://docs.nvisy.com)
- **Issues**: [GitHub Issues](https://github.com/nvisycom/elide/issues)
- **Email**: [support@nvisy.com](mailto:support@nvisy.com)
