# @nvisy/elide-wasm

In-browser PII detection and redaction, compiled to WebAssembly from the
[`elide`](https://github.com/nvisycom/elide) toolkit. The whole detect-and-redact
pipeline runs in the browser — no text leaves the page.

## Usage

```ts
import {
  createPatternRecognizer,
  createAnalyzer,
  createAnonymizer,
  redact,
} from "@nvisy/elide-wasm";

const recognizer = createPatternRecognizer({
  builtinPatterns: true,
  builtinDictionaries: true,
});
const analyzer = createAnalyzer([recognizer]); // consumes the recognizer
const anonymizer = createAnonymizer();

const { redacted, findings } = await redact(analyzer, anonymizer, text);
```

`createAnalyzer` consumes each recognizer handle. The analyzer and anonymizer
handles are reusable across many `redact` calls.

## Building

Built with [`wasm-pack`](https://github.com/rustwasm/wasm-pack) from the repo
root:

```sh
make wasm-pkg
```

This writes the bundler-target artifacts into `dist/`.
