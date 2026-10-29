# Containers

A DOCX holds body text, embedded images, and document properties: parts in
three modalities, each needing its own analyzer and anonymizer. The
`Orchestrator` (feature `engine`) drives them: register a pipeline per
modality, and it sends every part to the matching one.

```rust,no_run
use elide::prelude::operators::*;
use elide::prelude::*;
use elide::recognition::llm::LlmRecognizer;
# use elide::recognition::llm::backend::MockBackend;
use elide::recognition::pattern::PatternRecognizer;

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let bytes = std::fs::read("contract.docx")?;
#   let vision = MockBackend::<Image>::new();

    let registry = FormatRegistry::with_builtin();
    let mut document = registry.document("contract.docx", bytes).await?;

    let orchestrator = Orchestrator::new()
        .with_registry(registry)
        .with_scope(Scope::new().with_catalog(LabelCatalog::with_builtins()))
        .with_modality::<Text>(
            Analyzer::new()
                .with_recognizer(PatternRecognizer::builder().with_builtin_patterns().build()?),
            Anonymizer::new().with(Rule::fallback(Replace::new("[REDACTED]"))),
        )
        .with_modality::<Image>(
            // A vision model finds faces and signatures.
            Analyzer::new().with_recognizer(LlmRecognizer::with_default(vision)),
            Anonymizer::new()
                .with(Rule::label(builtins::FACE.to_ref(), Blur::new(12.0)))
                .with(Rule::fallback(Blackbox::default())),
        );

    orchestrator.anonymize(&mut document, &Directives::new()).await?;

    std::fs::write("contract.redacted.docx", document.document.encode()?.as_bytes())?;
    Ok(())
}
```

`registry.document` decodes a named file into an orchestrator `Document`.
`anonymize` analyzes every part, redacts it, and writes the parts back into
the container; `encode` then produces a complete replacement file.

Parts in a modality with no pipeline are left untouched. Here that is the
document properties; [Metadata](metadata.md) shows how to redact them too.

## Many documents

`analyze` and `anonymize` take one document or a slice of them, so a batch
runs in one call.

## Directives

`Directives` carry input for one run: a `Scope` that replaces the
orchestrator's default for this run, and regions to include or exclude per
modality.

```rust
use elide::prelude::*;

# fn main() -> Result<()> {
let directives = Directives::new()
    .with_scope(Scope::new().with_language(LanguageTag::parse("de")?));
# Ok(())
# }
```
