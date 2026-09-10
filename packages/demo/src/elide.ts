// Typed access to the wasm package.
//
// `@nvisy/elide-wasm` is built with wasm-pack's bundler target, so the wasm is
// instantiated by the bundler (via vite-plugin-wasm) on import — there is no
// manual init step. The pipeline is composed from opaque handles:
// `createPatternRecognizer` builds a recognizer from a config, `createAnalyzer`
// folds recognizers into an analyzer, `createAnonymizer` builds the redaction
// policy, and `redact` runs a composed analyzer + anonymizer over text.

export {
  createAnalyzer,
  createAnonymizer,
  createPatternRecognizer,
  redact,
} from "@nvisy/elide-wasm";
export type {
  AnalyzerHandle,
  AnonymizerHandle,
  Finding,
  PatternRecognizerConfig,
  RecognizerHandle,
  RedactionResult,
} from "@nvisy/elide-wasm";
