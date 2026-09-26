// `@nvisy/elide/recognizer` — entity recognizers.

import type { Branded, StageModality } from "./_shared.js";
import type { PatternRecognizerConfig } from "./dist/elide_wasm.js";

export type { PatternRecognizerConfig } from "./dist/elide_wasm.js";

/**
 * An opaque recognizer handle, branded with the modalities `M` it applies to.
 *
 * Built by {@link Recognizer.pattern} or {@link Recognizer.ner} (both
 * text-shaped, so every stage) and consumed by an analyzer's `recognize`. Same
 * contravariant brand as {@link Enricher}: a recognizer that supports more
 * modalities fits where fewer are required.
 */
export interface Recognizer<M extends StageModality = StageModality>
  extends Branded<M> {
  free(): void;
  [Symbol.dispose](): void;
}

/** Static constructors for a {@link Recognizer}. */
export const Recognizer: {
  /** A pattern recognizer; applies to every text-shaped modality. */
  pattern(config: PatternRecognizerConfig): Recognizer<StageModality>;
  /** A NER recognizer from a JS callback; applies to every text-shaped modality. */
  ner(callback: Function): Recognizer<StageModality>;
};
