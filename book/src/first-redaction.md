# Your first redaction

This chapter redacts a plain-text file: it finds the emails, phone numbers,
and payment cards in it, and replaces each with something safe to share.

## Finding entities

An `Analyzer` finds entities. Here it runs one recognizer, the built-in
patterns, and then two layers that tidy up what it found:

```rust
use elide::prelude::*;
use elide::recognition::pattern::PatternRecognizer;

# fn main() -> Result<()> {
let analyzer = Analyzer::<Text>::new()
    .with_recognizer(
        PatternRecognizer::builder()
            .with_builtin_patterns()
            .with_builtin_dictionaries()
            .build_context_enhanced()?,
    )
    .with_layer(ReconcileLayer::same_label(Merging::max()))
    .with_layer(FilterLayer::new().with_threshold(ConfidenceThreshold::BASELINE));
# Ok(())
# }
```

## Hiding them

An `Anonymizer` hides them. Its rules are tried in order, and the first that
matches an entity picks the operator that rewrites it:

```rust
use elide::prelude::operators::*;
use elide::prelude::*;

let anonymizer = Anonymizer::<Text>::new()
    .with(Rule::label(builtins::EMAIL_ADDRESS.to_ref(), Replace::new("[EMAIL]")))
    .with(Rule::label(builtins::PHONE_NUMBER.to_ref(), Replace::new("[PHONE]")))
    // Keep the last four digits of a card visible.
    .with(Rule::label(builtins::PAYMENT_CARD.to_ref(), Mask::stars().with_keep_suffix(4)))
    // Anything else found is removed.
    .with(Rule::fallback(Erase));
```

## The redaction

```rust
use elide::codec::DocumentPart;
use elide::prelude::operators::*;
use elide::prelude::*;
use elide::recognition::pattern::PatternRecognizer;

#[tokio::main]
async fn main() -> Result<()> {
    let input = "Contact Dana at dana.reed@example.com or +1 415 555 0100.";

    let registry = FormatRegistry::with_builtin();
    let mut document = registry.decode(input, "txt").await?;
#
#   let analyzer = Analyzer::<Text>::new()
#       .with_recognizer(
#           PatternRecognizer::builder()
#               .with_builtin_patterns()
#               .with_builtin_dictionaries()
#               .build_context_enhanced()?,
#       )
#       .with_layer(ReconcileLayer::same_label(Merging::max()))
#       .with_layer(FilterLayer::new().with_threshold(ConfidenceThreshold::BASELINE));
#   let anonymizer = Anonymizer::<Text>::new()
#       .with(Rule::label(builtins::EMAIL_ADDRESS.to_ref(), Replace::new("[EMAIL]")))
#       .with(Rule::label(builtins::PHONE_NUMBER.to_ref(), Replace::new("[PHONE]")))
#       .with(Rule::label(builtins::PAYMENT_CARD.to_ref(), Mask::stars().with_keep_suffix(4)))
#       .with(Rule::fallback(Erase));

    let scope = Scope::new()
        .with_language(LanguageTag::parse("en")?)
        .with_catalog(LabelCatalog::with_builtins());

    let DocumentPart::Stream { handle, .. } = &mut document.parts_mut()[0] else {
        unreachable!("a text file is one text stream");
    };
    let stream = handle.downcast_mut::<Text>().expect("a text stream");

    let mut entities = analyzer.analyze_stream(stream, &scope).await?.entities;
    anonymizer.anonymize(stream, &mut entities, &scope).await?;

    let redacted = document.encode()?;
    assert_eq!(redacted.as_bytes(), b"Contact Dana at [EMAIL] or [PHONE].");
    Ok(())
}
```

Step by step:

1. **The registry** decodes the input by its extension. A text file decodes
   to a document with one part: a stream of text.
2. **The scope** says what the caller knows about this input: that it is
   English, and which labels are in play. Recognizers read it.
3. **`analyze_stream`** reads the stream chunk by chunk, runs every
   recognizer on each chunk, and returns the entities in the document's own
   coordinates, after the layers have reconciled them.
4. **`anonymize`** picks an operator for each entity and writes the
   replacements back into the stream.
5. **`encode`** writes the document back out. Only the redacted spans
   changed.

"Dana" was not redacted: names need a model, not a pattern. See
[Named-entity models](ner.md).

The repository has runnable versions of this and of a DOCX redaction, with
offline mock models:

```sh
cargo run -p elide-examples --bin redact-txt
cargo run -p elide-examples --bin redact-docx
```
