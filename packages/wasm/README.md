# @nvisy/elide

[![npm](https://img.shields.io/npm/v/@nvisy/elide?style=flat-square)](https://www.npmjs.com/package/@nvisy/elide)
[![Build](https://img.shields.io/github/actions/workflow/status/nvisycom/elide/build.yml?branch=main&label=build%20%26%20test&style=flat-square)](https://github.com/nvisycom/elide/actions/workflows/build.yml)

In-browser PII detection and redaction, compiled to WebAssembly.

The [Elide](https://github.com/nvisycom/elide) detect-and-redact pipeline runs
entirely in the browser: Elide itself sends no text, image, or audio anywhere,
and whether a model you plug in does is up to its callback. It
covers text, tabular, image, and audio, and the DOCX, PPTX, XLSX, and PDF
documents that hold them, combining deterministic patterns with
browser-supplied NER, LLM, OCR, and speech-to-text models, and rewrites the
input with a policy-driven anonymizer.

## Installation

```bash
npm install @nvisy/elide
```

The package ships a bundler-target WebAssembly module; use it with a bundler
that supports WebAssembly imports (Vite, webpack, Rollup).

## Quick Start

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
new TextDecoder().decode(redacted); // -> "Email me at [EMAIL]."
entities[0].value;    // -> "dana.reed@example.com", what it matched
entities[0].location; // -> { kind: "text", start, end, utf16: { start, end } }
entities[0].sources;  // -> which recognizers (and models) found it
entities[0].redaction; // -> { operator: "replace", leakProfile: "irrecoverable" }
```

An `Orchestrator` pairs a detect side (an `Analyzer`) with an optional redact
side (an `Anonymizer`) per modality, then runs the pipeline over a blob with
`redact`, dispatching by the format hint: a file extension, or nothing, to
recognize a binary format by its leading bytes. Omit the anonymizer to use the
default policy. The orchestrator is reusable across many `redact` calls.

Browser-supplied models — NER, an LLM, language detection, OCR,
speech-to-text — are async callbacks, each wrapped in a `Backend` that can be
rate- and concurrency-limited, fail over, or route by language, detected or
asserted with `Orchestrator.withLanguages`:

```typescript
import {
  Analyzer, Backend, Enricher, Layer, Recognizer,
  type LlmCallback, type OcrCallback,
} from "@nvisy/elide/analyzer";

const image = Analyzer.image()
  .enrich(Enricher.ocr(Backend.callback<OcrCallback>(runOcr))) // Promise<OcrRegion[]>
  .recognize(Recognizer.pattern({ builtinPatterns: true }))
  .layer(Layer.filter(0.5));

// The recognizer renders the prompt and places the answer in the text; the
// callback only calls the model, asking for output in `request.schema`.
const llm = Backend.callback<LlmCallback>(async (prompt, request) =>
  callModel(prompt, request.schema), // Promise<{ entities, model? }>
)
  .rateLimited({ count: 60, intervalMs: 60_000 })
  .concurrencyLimited(4)
  .fallbackRouted(Backend.callback<LlmCallback>(callSmallerModel));
const text = Analyzer.text().recognize(Recognizer.llm(llm));
```

## Features

- **Every modality in the browser** — text, tabular, image (EXIF scrub + OCR
  pixel redaction), and audio, from one API.
- **Documents** — DOCX, PPTX, XLSX, and PDF, each decoded into the modalities
  it holds; a PDF entity is located by its page and boxes.
- **Compile-time type-safety** — each handle is branded with the modalities it
  applies to, so a wrong pairing (an OCR enricher on a text analyzer) is a
  TypeScript error, not a runtime surprise.
- **Bring your own models** — NER, LLM, language detection, OCR, and STT are
  async JavaScript callbacks; wire a remote endpoint, `transformers.js`, or a WebGPU model, and
  bound it with rate and concurrency limits, a fallback, and language routes.
- **Configurable pipelines** — reconcile/filter layers on the detect side,
  label/fallback rules on the redact side, with operators from `replace`,
  `mask`, and `cover` to `fake`, `hmac`, `encrypt`, `generalizeDate`, `blur`,
  and `beep`.
- **No requests of its own** — Elide makes no network requests and needs no
  filesystem or worker threads; only your callbacks reach out, if they do.

## Subpath exports

| Import | Provides |
| --- | --- |
| `@nvisy/elide` | `Orchestrator`, `ElideError`, the modality and result types |
| `@nvisy/elide/analyzer` | `Analyzer`, `Recognizer`, `Enricher`, `Layer`, `Backend` |
| `@nvisy/elide/anonymizer` | `Anonymizer`, `Rule`, `Operator`, `Label` |

## Building

Built with [`wasm-pack`](https://github.com/rustwasm/wasm-pack) from the repo
root:

```sh
make wasm-pkg
```

This writes the bundler-target artifacts into `dist/` and regenerates the
`Label` constants from the toolkit's built-in catalog.

## Deployment

If you need the full multimodal redaction platform rather than the standalone
in-browser engine, see [`@nvisy/sdk`](https://www.npmjs.com/package/@nvisy/sdk),
the TypeScript client for [Nvisy](https://nvisy.com/).

## License

Apache-2.0, see [LICENSE](https://github.com/nvisycom/elide/blob/main/LICENSE).

## Support

- **Issues**: [github.com/nvisycom/elide/issues](https://github.com/nvisycom/elide/issues)
- **Email**: [support@nvisy.com](mailto:support@nvisy.com)
