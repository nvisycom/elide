# @nvisy/elide

[![npm](https://img.shields.io/npm/v/@nvisy/elide?style=flat-square)](https://www.npmjs.com/package/@nvisy/elide)
[![Build](https://img.shields.io/github/actions/workflow/status/nvisycom/elide/build.yml?branch=main&label=build%20%26%20test&style=flat-square)](https://github.com/nvisycom/elide/actions/workflows/build.yml)

In-browser PII detection and redaction, compiled to WebAssembly.

The [Elide](https://github.com/nvisycom/elide) detect-and-redact pipeline runs
entirely in the browser — no text, image, or audio ever leaves the page. It
covers text, tabular, image, and audio, combining deterministic patterns with
browser-supplied NER, OCR, and speech-to-text models, and rewrites the input
with a policy-driven anonymizer.

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
    .rule(Rule.fallback(Operator.erase())),
);

const bytes = new TextEncoder().encode("Email me at dana.reed@example.com.");
const { redacted, entities } = await orchestrator.redact(bytes, "txt");
new TextDecoder().decode(redacted); // -> "Email me at [EMAIL]."
entities; // -> the detected entities, each with a label and location
```

An `Orchestrator` pairs a detect side (an `Analyzer`) with an optional redact
side (an `Anonymizer`) per modality, then runs the pipeline over a blob with
`redact`, dispatching by the format hint. Omit the anonymizer to use the default
policy. The orchestrator is reusable across many `redact` calls.

Browser-supplied models — NER, OCR, speech-to-text — are passed as async
callbacks; language detection is built in:

```typescript
import { Analyzer, Recognizer, Enricher, Layer } from "@nvisy/elide/analyzer";

const analyzer = Analyzer.image()
  .enrich(Enricher.ocr(async (image) => runOcr(image))) // Promise<OcrBlock[]>
  .recognize(Recognizer.pattern({ builtinPatterns: true }))
  .layer(Layer.filter(0.5));
```

## Features

- **Every modality in the browser** — text, tabular, image (EXIF scrub + OCR
  pixel redaction), and audio, from one API.
- **Compile-time type-safety** — each handle is branded with the modalities it
  applies to, so a wrong pairing (an OCR enricher on a text analyzer) is a
  TypeScript error, not a runtime surprise.
- **Bring your own models** — NER, OCR, and STT are async JavaScript callbacks;
  wire a remote endpoint, `transformers.js`, or a WebGPU model.
- **Configurable pipelines** — reconcile/filter layers on the detect side,
  label/fallback rules with `replace`/`mask`/`erase`/… operators on the redact
  side.
- **Nothing leaves the page** — no network, no filesystem, no worker threads.

## Subpath exports

| Import | Provides |
| --- | --- |
| `@nvisy/elide` | `Orchestrator`, `ElideError`, the modality and result types |
| `@nvisy/elide/analyzer` | `Analyzer`, `Recognizer`, `Enricher`, `Layer` |
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
