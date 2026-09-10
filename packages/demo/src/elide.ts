// Typed wrapper over the `@nvisy/elide-wasm` package.
//
// The package is built with wasm-pack's bundler target, so the wasm is
// instantiated by the bundler (via vite-plugin-wasm) on import — there is no
// manual init step. This module owns the pipeline lifecycle: it builds the
// analyzer + anonymizer handles from the selected sources, reuses them across
// redactions, and frees them safely even when the sources change mid-redaction.

import {
  createAnalyzer,
  createAnonymizer,
  createPatternRecognizer,
  redact,
  type AnalyzerHandle,
  type AnonymizerHandle,
  type RedactionResult,
} from "@nvisy/elide-wasm";

export type { Finding, RedactionResult } from "@nvisy/elide-wasm";

/** Which built-in recognizer sources the pipeline draws on. */
export type Sources = { patterns: boolean; dictionaries: boolean };

type Handles = { analyzer: AnalyzerHandle; anonymizer: AnonymizerHandle };

function build(sources: Sources): Handles {
  const recognizer = createPatternRecognizer({
    builtinPatterns: sources.patterns,
    builtinDictionaries: sources.dictionaries,
  });
  return {
    analyzer: createAnalyzer([recognizer]),
    anonymizer: createAnonymizer(),
  };
}

function free(handles: Handles): void {
  handles.analyzer.free();
  handles.anonymizer.free();
}

/**
 * Owns the analyzer + anonymizer handles that live in wasm memory.
 *
 * The handles only depend on the selected sources, so they are built once and
 * reused across redactions; {@link invalidate} drops them so the next
 * {@link redact} rebuilds from fresh sources. Freeing is deferred while a
 * `redact` call is in flight — the handles it borrows must outlive the awaited
 * call, or the wasm boundary would see a use-after-free.
 */
export class Pipeline {
  #handles: Handles | null = null;
  #inFlight: Handles | null = null;

  constructor(private readonly sources: () => Sources) {}

  /** Drop the current handles so the next redaction rebuilds from new sources. */
  invalidate(): void {
    // Free now if idle; if a redaction is borrowing the handles, `redact` frees
    // them when it settles.
    if (this.#handles && this.#handles !== this.#inFlight) free(this.#handles);
    this.#handles = null;
  }

  /** Detect and redact `input`, building the handles on first use. */
  async redact(input: string): Promise<RedactionResult> {
    this.#handles ??= build(this.sources());

    // Hold the handles for the whole `await`, so an `invalidate` mid-redaction
    // can drop `#handles` without freeing what `redact` is still borrowing.
    const active = this.#handles;
    this.#inFlight = active;
    try {
      return await redact(active.analyzer, active.anonymizer, input);
    } finally {
      this.#inFlight = null;
      // If `invalidate` ran mid-redaction, free the now-detached handles.
      if (this.#handles !== active) free(active);
    }
  }
}
