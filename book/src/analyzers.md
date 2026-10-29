# Analyzers and scopes

An `Analyzer<M>` finds the entities in one modality `M`. It is configuration:
build it once and reuse it for every input. It runs three kinds of stages, in
order:

1. **Enrichers** add what the recognizers need: the text of an image, the
   transcript of a clip, the language of a text.
2. **Recognizers** propose entities. They run concurrently.
3. **Layers** reshape the proposals into one consistent set.

```rust
use elide::prelude::*;
use elide::recognition::pattern::PatternRecognizer;

# fn main() -> Result<()> {
let analyzer = Analyzer::<Text>::new()
    .with_recognizer(PatternRecognizer::builder().with_builtin_patterns().build()?)
    .with_layer(ReconcileLayer::same_label(Merging::max()))
    .with_layer(FilterLayer::new().with_threshold(ConfidenceThreshold::BASELINE));
# Ok(())
# }
```

## The scope

Each call takes a `Scope`: what the caller knows about this input. Every
recognizer, and every anonymizer rule, can read it.

```rust
use elide::prelude::*;
use elide::primitive::CountryCode;

# fn main() -> Result<()> {
let scope = Scope::new()
    .with_language(LanguageTag::parse("pl")?)
    .with_country(CountryCode::from_alpha2("PL").expect("a valid country code"))
    .with_catalog(LabelCatalog::with_builtins());
# Ok(())
# }
```

- **Languages** turn on language-specific patterns and dictionaries, pick a
  model variant, and drive [language routing](routing.md). Without one, an
  enricher such as `lingua` can detect it.
- **Countries** turn on national formats: identity numbers, bank accounts,
  phone numbers.
- **The catalog** is the set of labels in play, with their descriptions.
  Recognizers emit only labels the catalog declares, and zero-shot models
  read the descriptions to know what to look for.
- **Tags, purpose, and audience** describe the call itself, for recognizers
  and rules that want them.

## Running an analysis

| Method | Analyzes |
|---|---|
| `analyze(data, &scope)` | One payload in memory, such as a `TextData` |
| `analyze_stream(stream, &scope)` | A decoded document's stream, chunk by chunk |
| `analyze_with`, `analyze_stream_with` | The same, with regions to include or exclude |

```rust
use elide::modality::text::TextData;
use elide::prelude::*;
use elide::recognition::pattern::PatternRecognizer;

# #[tokio::main]
# async fn main() -> Result<()> {
# let analyzer = Analyzer::<Text>::new()
#     .with_recognizer(PatternRecognizer::builder().with_builtin_patterns().build()?);
let text = TextData::new("Card: 4111 1111 1111 1111");
let scope = Scope::new().with_catalog(LabelCatalog::with_builtins());
let analysis = analyzer.analyze(text, &scope).await?;

for entity in &analysis.entities {
    println!("{} ({})", entity.label.as_str(), entity.confidence);
}
# Ok(())
# }
```

The returned entities are in the caller's coordinates: for a stream, offsets
into the whole document, not into the chunk they were found in. The
`Analysis` also carries the enrichment it produced, such as an OCR layout, so
a later run can reuse it instead of paying for it again.

An analysis fails if any recognizer fails: a recognizer that errored has not
vouched that the content is clean. To trade that for availability, see
[Recognizer decorators](recognizer-decorators.md).
