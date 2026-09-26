// `@nvisy/elide/enricher` — pre-recognition enrichers.

import type { Branded, StageModality, Image, Audio } from "./_shared.js";
import type { LanguageSet } from "./dist/elide_wasm.js";

export type { LanguagePreset, LanguageSet } from "./dist/elide_wasm.js";

/**
 * An opaque enricher handle, branded with the modalities `M` it applies to.
 *
 * Built by {@link Enricher.language} (every modality), {@link Enricher.ocr}
 * ({@link Image}), or {@link Enricher.stt} ({@link Audio}), and consumed by an
 * analyzer's `enrich`. Passing one to a stage it does not support is a type
 * error.
 */
export interface Enricher<M extends StageModality = StageModality>
  extends Branded<M> {
  free(): void;
  [Symbol.dispose](): void;
}

/** Static constructors for an {@link Enricher}. */
export const Enricher: {
  /** The built-in language-detection enricher; applies to every modality. */
  language(languages: LanguageSet): Enricher<StageModality>;
  /** An OCR enricher from a JS callback; applies to the image modality. */
  ocr(callback: Function): Enricher<Image>;
  /** An STT enricher from a JS callback; applies to the audio modality. */
  stt(callback: Function): Enricher<Audio>;
};
