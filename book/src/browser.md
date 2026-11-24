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
    .rule(Rule.fallback(Operator.remove())),
);

const bytes = new TextEncoder().encode("Email me at dana.reed@example.com.");
const { redacted, entities } = await orchestrator.redact(bytes, "txt");
new TextDecoder().decode(redacted); // "Email me at [EMAIL]."
```

Without an anonymizer, a modality uses the default policy. An orchestrator
can redact any number of blobs.

Each entity says what it matched (`value`), where (`location`, discriminated
by `kind`; a text span in bytes and in UTF-16 code units, the offsets a
JavaScript string indexes by), which recognizers and models found it
(`sources`), and which operator redacted it and how much that leaks
(`redaction`). Offsets point into the decoded text: for a document, the text
its codec extracted, and for a PDF, the page's text.

Documents work the same way: a DOCX, PPTX, XLSX, or PDF decodes into the
modalities it holds, and each is redacted by that modality's stage. The hint
names the format by its extension; leave it out and a binary format is
recognized by its leading bytes. A PDF entity's location carries the page and
the boxes it is drawn in.

## Models in the browser

NER, LLM, language detection, OCR, and speech-to-text models are async
callbacks, so they can call an endpoint, run `transformers.js`, use a WebGPU
model, or use what the browser ships, like its `LanguageDetector`. Each
callback is wrapped in a `Backend`:

```typescript
import {
  Analyzer, Backend, Enricher, Layer, Recognizer, type OcrCallback,
} from "@nvisy/elide/analyzer";

const analyzer = Analyzer.image()
  .enrich(Enricher.ocr(Backend.callback<OcrCallback>(runOcr))) // Promise<OcrRegion[]>
  .recognize(Recognizer.pattern({ builtinPatterns: true }))
  .layer(Layer.filter(0.5));
```

A backend takes the same decorators as a Rust one: `rateLimited` admits calls
at a rate, `concurrencyLimited` caps the calls in flight, `fallbackRouted` sends a
call whose callback throws to another backend, and `languageRouted` with its
`route`s sends requests in one language to another backend: the language
`Enricher.language` detects, or the one asserted with
`Orchestrator.withLanguages(["de"])`.

The LLM recognizer works as it does in Rust: it renders the prompt, from the
built-in wording or a Jinja2 template, and places each candidate the model
returns in the text by its `context`. The callback gets the prompt and a
request carrying the answer's JSON Schema, the language, and on an image stage
the image as PNG, and returns `{ entities, model? }`:

```typescript
import { Backend, Recognizer, type LlmCallback } from "@nvisy/elide/analyzer";

const llm = Recognizer.llm(
  Backend.callback<LlmCallback>((prompt, request) => callModel(prompt, request.schema))
    .rateLimited({ count: 60, intervalMs: 60_000 }),
);
```

Each handle is typed with the modalities it applies to, so an OCR enricher
on a text analyzer, or an LLM recognizer on a tabular one, is a type error.

Every redaction operator but pseudonymization is available: besides
`replace`, `mask`, `remove`, `cover`, and `keep`, there are `truncate`,
`clamp`, `generalizeDate`, `fake`, `hash`, `hmac`, and `encrypt` for text and
cells, `dropRow` and `dropColumn` for tables, `blur` and `pixelate` for
images, and `beep` for audio. Keys for `hmac` and `encrypt` are passed as
bytes.

| Import | Provides |
|---|---|
| `@nvisy/elide` | `Orchestrator`, `ElideError`, the modality and result types |
| `@nvisy/elide/analyzer` | `Analyzer`, `Recognizer`, `Enricher`, `Layer`, `Backend` |
| `@nvisy/elide/anonymizer` | `Anonymizer`, `Rule`, `Operator`, `Label` |

## What the browser build leaves out

Text, tables, images, audio, and the documents that hold them all work in
the browser. Left out are the parts that need a native library or a network
client: MP3, PDF page rendering, and `RigBackend`, whose place the LLM
callback takes. Without page rendering, a PDF page whose text can't be placed
(in a font whose widths can't be known, or written vertically) is refused.
Language detection is a callback rather than compiled-in models, which would
make the module several times larger.

To build the package and the demo from the repository:

```sh
make wasm-pkg    # the package, into packages/wasm/dist
make wasm-dev    # the demo, with hot reload
```
