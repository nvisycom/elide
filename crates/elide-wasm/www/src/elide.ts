// Typed access to the wasm-bindgen module.
//
// The Rust structs derive `Tsify`, so `redact_text` is already typed as
// `Promise<RedactionResult>` and the `Finding` / `RedactionResult` interfaces
// come straight from the generated package, with no hand-written mirror types
// and no `any`. This module only adds one-time initialization.

import init, { redact_text } from "elide-wasm";

export type { Finding, RedactionResult } from "elide-wasm";

let ready: Promise<void> | null = null;

/** Load and initialize the wasm module once; subsequent calls reuse it. */
export function initElide(): Promise<void> {
  if (!ready) {
    ready = init().then(() => undefined);
  }
  return ready;
}

/** Detect and redact the personal data in `input`. The `patterns` and
 * `dictionaries` flags select which recognizer sources run. Initializes the
 * module on first use. */
export async function redactText(
  input: string,
  patterns: boolean,
  dictionaries: boolean,
) {
  await initElide();
  return redact_text(input, patterns, dictionaries);
}
