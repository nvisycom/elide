# WebAssembly

Elide compiles to WebAssembly and ships on npm as
[`@nvisy/elide`](https://www.npmjs.com/package/@nvisy/elide). It runs
entirely in the page: Elide itself sends no text, image, or audio anywhere.
The models you plug in are your callbacks, so whether input leaves the
browser is up to them: a callback that calls a remote endpoint sends it
there. Try it in the [live demo](https://nvisycom.github.io/elide/demo/),
which uses no remote models.

```sh
npm install @nvisy/elide
```

The package is a bundler-target module; use it with a bundler that supports
WebAssembly imports, such as Vite, webpack, or Rollup.

## Redacting

The JavaScript API mirrors the Rust one. An `Orchestrator` pairs an
`Analyzer` with an `Anonymizer` per modality, and redacts a blob by its
format:

```typescript
import { Orchestrator } from "@nvisy/elide";
import { Analyzer, Recognizer } from "@nvisy/elide/analyzer";
import { Anonymizer, Rule, Operator, Label } from "@nvisy/elide/anonymizer";

const orchestrator = new Orchestrator().with(
  Analyzer.text().recognize(Recognizer.pattern({
    builtinPatterns: true,
    builtinDictionaries: true,
  })),
  Anonymizer.text()
    .rule(Rule.label(Label.EmailAddress, Operator.replace("[EMAIL]")))
    .rule(Rule.fallback(Operator.erase())),
);

const bytes = new TextEncoder().encode("Email me at dana.reed@example.com.");
const { redacted, entities } = await orchestrator.redact(bytes, "txt");
new TextDecoder().decode(redacted); // "Email me at [EMAIL]."
```

Without an anonymizer, a modality uses the default policy. An orchestrator
can redact any number of blobs.

## Models in the browser

Language detection is built in. NER, OCR, and speech-to-text models are
async callbacks, so they can call an endpoint, run `transformers.js`, or use
a WebGPU model:

```typescript
import { Analyzer, Recognizer, Enricher, Layer } from "@nvisy/elide/analyzer";

const analyzer = Analyzer.image()
  .enrich(Enricher.ocr(async (image) => runOcr(image))) // Promise<OcrRegion[]>
  .recognize(Recognizer.pattern({ builtinPatterns: true }))
  .layer(Layer.filter(0.5));
```

Each handle is typed with the modalities it applies to, so an OCR enricher
on a text analyzer is a type error.

| Import | Provides |
|---|---|
| `@nvisy/elide` | `Orchestrator`, `ElideError`, the modality and result types |
| `@nvisy/elide/analyzer` | `Analyzer`, `Recognizer`, `Enricher`, `Layer` |
| `@nvisy/elide/anonymizer` | `Anonymizer`, `Rule`, `Operator`, `Label` |

## What the browser build leaves out

Text, tables, images, and audio all work in the browser. Left out are the
parts that need a native library or a network client: MP3 decoding, PDF
page rendering, and `RigBackend`.

To build the package and the demo from the repository:

```sh
make wasm-pkg    # the package, into packages/wasm/dist
make wasm-dev    # the demo, with hot reload
```
