# Introduction

Elide is a toolkit for finding and removing sensitive data (PII, PHI, secrets)
from text, images, audio, and tabular data, written in Rust. You assemble a
pipeline from its pieces: *recognizers* that find entities, *layers* that
reconcile what they found, and *operators* that hide each one. Elide then
rewrites the original file, changing only what was redacted.

```rust
use elide::modality::text::TextData;
use elide::prelude::*;
use elide::recognition::pattern::PatternRecognizer;

# #[tokio::main]
# async fn main() -> Result<()> {
let analyzer = Analyzer::<Text>::new()
    .with_recognizer(PatternRecognizer::builder().with_builtin_patterns().build()?);

let text = TextData::new("Write to dana.reed@example.com");
let scope = Scope::new().with_catalog(LabelCatalog::with_builtins());
let analysis = analyzer.analyze(text, &scope).await?;

assert_eq!(analysis.entities[0].label, builtins::EMAIL_ADDRESS.to_ref());
# Ok(())
# }
```

## What Elide does for you

- **One entity model across modalities**: a span of text, a cell, a region of
  pixels, an interval of audio, a metadata field. A recognizer or operator
  written once serves every format.
- **Several detection techniques at once**: regex, dictionary, and checksum
  patterns; named-entity models; LLMs and VLMs. Their findings are reconciled
  into one deduplicated set.
- **Context-aware scoring**: nearby keywords raise the confidence of an
  ambiguous match.
- **Redaction operators**: mask, replace, truncate, hash, encrypt, or
  pseudonymize text; blur, pixelate, or black out image regions; silence or
  beep audio; drop rows and columns. Encryption and pseudonymization can be
  reversed by whoever holds the key.
- **Format codecs**: read and rewrite TXT, CSV, JSON, XML, HTML, PDF, DOCX,
  PPTX, XLSX, PNG, JPEG, TIFF, WAV, and MP3, leaving every byte that was not
  redacted intact.
- **A full audit trail**: every entity records how it was found, scored, and
  hidden, and the trail verifies.

Elide is a library, not a service. It does not host a server, store audit
trails, or schedule work; those belong to the application that embeds it.

## The crates

Most projects depend only on `elide`, which re-exports the rest behind
feature flags:

| Crate | Contains |
|---|---|
| `elide` | The facade: everything below, behind features |
| `elide-core` | The entity model, modalities, and the recognizer, operator, and backend contracts |
| `elide-detection` | The `Analyzer`, its layers, and the recognizer decorators |
| `elide-pattern` | Regex, dictionary, and checksum recognition |
| `elide-ner` | Named-entity recognition over a model backend |
| `elide-llm` | LLM and VLM recognition, and the `RigBackend` |
| `elide-redaction` | The `Anonymizer` and `Deanonymizer` |
| `elide-operator` | The redaction operators |
| `elide-backend` | Backend decorators: metering, rate limits, routing |
| `elide-engine` | The `Orchestrator` for multi-part documents |
| `elide-image`, `elide-audio` | The image and audio modalities, OCR, and speech-to-text |
| `elide-plain`, `elide-office`, `elide-pdf` | Format codecs |

The API reference is on [docs.nvisy.com](https://docs.nvisy.com). This book is
the guide: it explains how the pieces fit together, chapter by chapter.

> **The API is not stable yet.** Public types and configuration may change
> between releases. Pin a specific commit if you depend on Elide in
> production.
