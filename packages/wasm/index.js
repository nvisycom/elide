// `@nvisy/elide` — the pipeline entry.
//
// Re-exports the wasm-bindgen runtime unchanged; all branding lives in the
// colocated `.d.ts` files, which are types-only, so this file is a plain
// pass-through. The detect and redact stages live under the `/analyzer` and
// `/anonymizer` subpaths. The `lingua` crate's leaked wasm-bindgen classes
// (`LanguageDetector`, …) are intentionally not re-exported.

export {
  ElideError,
  ElideErrorKind,
  Orchestrator,
  start,
} from "./dist/elide_wasm.js";
