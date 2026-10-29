# Images and OCR

Sensitive data in an image is either text (a name on a scanned form, a card
number in a screenshot) or visual (a face, a signature). Elide finds the
first with OCR and the text recognizers, and the second with a vision model.
Either way the entity is a region of pixels, and an operator paints over it.

## Text in images

An `OcrEnricher` (feature `ocr`) reads the text in an image and its position
before the recognizers run. Every text recognizer then works on the image as
it would on text, and its findings are mapped back to the pixels the text
covers:

```rust
use elide::enrichment::ocr::OcrEnricher;
use elide::prelude::operators::*;
use elide::prelude::*;
use elide::recognition::pattern::PatternRecognizer;

# fn main() -> Result<()> {
# let ocr = elide::enrichment::ocr::MockBackend::new();
let analyzer = Analyzer::<Image>::new()
    .with_enricher(OcrEnricher::new(ocr))
    .with_recognizer(PatternRecognizer::builder().with_builtin_patterns().build()?);

let anonymizer = Anonymizer::<Image>::new().with(Rule::fallback(Blackbox::default()));
# Ok(())
# }
```

The OCR engine is a [backend](backends.md). `OcrsBackend`, a pure-Rust OCR
engine, comes with the `ocrs` feature of `elide-image`; it loads its two
model files once, from a directory you give it. Any other engine, local or
hosted, is a small backend of your own.

## Faces, signatures, handwriting

A vision model finds what is not text. An `LlmRecognizer` over a
`RigBackend<Image>` sends the image to a multimodal model and asks for the
regions of faces, signatures, and other visual identifiers; see
[Language models](llm.md).

```rust
use elide::prelude::operators::*;
use elide::prelude::*;

let anonymizer = Anonymizer::<Image>::new()
    .with(Rule::label(builtins::FACE.to_ref(), Blur::new(12.0)))
    .with(Rule::label(builtins::SIGNATURE.to_ref(), Blackbox::default()))
    .with(Rule::fallback(Pixelate::new(16)));
```

## Image operators

| Operator | Effect |
|---|---|
| `Blackbox` | Fills the region with a solid color, black by default |
| `Blur::new(sigma)` | Gaussian blur; a larger sigma blurs more |
| `Pixelate::new(block_size)` | Replaces the region with blocks of that size |

A blur or pixelation can sometimes be reversed on small text. Prefer
`Blackbox` for text found by OCR.

Images inside a DOCX, PPTX, or PDF go through the same pipeline when the
[orchestrator](containers.md) has one registered for `Image`.
