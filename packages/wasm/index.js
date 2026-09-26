// `@nvisy/elide` — the pipeline entry.
//
// Re-exports the wasm-bindgen runtime unchanged; all branding lives in the
// colocated `.d.ts` files, which are types-only, so this file is a plain
// pass-through. The building blocks live under the `/enricher`, `/recognizer`,
// and `/layer` subpaths. The `lingua` crate's leaked wasm-bindgen classes
// (`LanguageDetector`, …) are intentionally not re-exported.

export {
  Analyzer,
  ElideError,
  ElideErrorKind,
  Orchestrator,
  start,
} from "./dist/elide_wasm.js";
