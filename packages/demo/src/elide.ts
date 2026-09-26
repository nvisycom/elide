// Typed wrapper over the `@nvisy/elide` package.
//
// The package is built with wasm-pack's bundler target, so the wasm is
// instantiated by the bundler (via vite-plugin-wasm) on import — there is no
// manual init step. This module owns the pipeline lifecycle: it builds the
// pipeline handle from the selected sources, reuses it across redactions, and
// frees it safely even when the sources change mid-redaction.

import { Orchestrator, type Finding } from "@nvisy/elide";
import { Analyzer, Recognizer } from "@nvisy/elide/analyzer";
import { Anonymizer, Label, Operator, Rule } from "@nvisy/elide/anonymizer";

export type { Finding } from "@nvisy/elide";

/** A redaction result with the redacted bytes decoded back to text. */
export type TextRedaction = { redacted: string; findings: Finding[] };

/** Which built-in recognizer sources the pipeline draws on. */
export type Sources = { patterns: boolean; dictionaries: boolean };

/**
 * A readable text policy: a labelled token per common kind, a masked tail for
 * payment cards, and full erasure for anything else detected.
 */
function policy(): Anonymizer<"text"> {
  return Anonymizer.text()
    .rule(Rule.label(Label.EmailAddress, Operator.replace("[EMAIL]")))
    .rule(Rule.label(Label.PhoneNumber, Operator.replace("[PHONE]")))
    .rule(Rule.label(Label.Url, Operator.replace("[URL]")))
    .rule(Rule.label(Label.PaymentCard, Operator.mask({ keepSuffix: 4 })))
    .rule(Rule.fallback(Operator.erase()));
}

function build(sources: Sources): Orchestrator {
  const recognizer = Recognizer.pattern({
    builtinPatterns: sources.patterns,
    builtinDictionaries: sources.dictionaries,
  });
  // A text-only pipeline: one recognizer, the default layers, the policy above.
  return new Orchestrator().with(
    Analyzer.text().recognize(recognizer),
    policy(),
  );
}

/**
 * Owns the pipeline handle that lives in wasm memory.
 *
 * The handle only depends on the selected sources, so it is built once and
 * reused across redactions; {@link invalidate} drops it so the next
 * {@link redact} rebuilds from fresh sources. Freeing is deferred while a
 * `redact` call is in flight — the handle it borrows must outlive the awaited
 * call, or the wasm boundary would see a use-after-free.
 */
export class Pipeline {
  #handle: Orchestrator | null = null;
  #inFlight: Orchestrator | null = null;

  constructor(private readonly sources: () => Sources) {}

  /** Drop the current handle so the next redaction rebuilds from new sources. */
  invalidate(): void {
    // Free now if idle; if a redaction is borrowing the handle, `redact` frees
    // it when it settles.
    if (this.#handle && this.#handle !== this.#inFlight) this.#handle.free();
    this.#handle = null;
  }

  /** Detect and redact `input` text, building the handle on first use. */
  async redact(input: string): Promise<TextRedaction> {
    this.#handle ??= build(this.sources());

    // Hold the handle for the whole `await`, so an `invalidate` mid-redaction
    // can drop `#handle` without freeing what `redact` is still borrowing.
    const active = this.#handle;
    this.#inFlight = active;
    try {
      const bytes = new TextEncoder().encode(input);
      const result = await active.redact(bytes, "txt");
      return {
        redacted: new TextDecoder().decode(result.redacted),
        findings: result.findings,
      };
    } finally {
      this.#inFlight = null;
      // If `invalidate` ran mid-redaction, free the now-detached handle.
      if (this.#handle !== active) active.free();
    }
  }
}
