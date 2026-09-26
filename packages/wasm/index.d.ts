// `@nvisy/elide` — the pipeline entry.
//
// Branded public types over the wasm-bindgen output. wasm-bindgen erases
// generics, so every handle it emits is one un-parameterized class; this overlay
// layers a phantom modality brand so a mismatched wiring (an OCR enricher on a
// text analyzer) is a TypeScript error. The brand exists only in the types;
// `index.js` re-exports the runtime unchanged. The building blocks live under
// the `/enricher`, `/recognizer`, and `/layer` subpaths.

import type { Branded, StageModality, Text, Tabular, Image, Audio } from "./_shared.js";
import type { RedactionResult } from "./_shared.js";
import type { Enricher } from "./enricher.js";
import type { Recognizer } from "./recognizer.js";
import type { Layer } from "./layer.js";
import type { Anonymizer } from "./anonymizer.js";

export { ElideError, ElideErrorKind } from "./dist/elide_wasm.js";
export type {
  Text,
  Tabular,
  Image,
  Audio,
  Metadata,
  Modality,
  StageModality,
  Finding,
  RedactionResult,
  ByteRange,
} from "./_shared.js";

/**
 * The detect side of a modality's stage, branded with its modality `M`,
 * mirroring the toolkit's `Analyzer` builder.
 *
 * Built with {@link Analyzer.text}/{@link Analyzer.tabular}/{@link Analyzer.image}/
 * {@link Analyzer.audio}, then extended with {@link enrich}, {@link recognize},
 * and {@link layer}. The `M` brand makes a mismatched handle — an OCR enricher on
 * a text analyzer — a compile error. Hand it to {@link Orchestrator.with}.
 */
export interface Analyzer<M extends StageModality = StageModality>
  extends Branded<M> {
  enrich(enricher: Enricher<M>): Analyzer<M>;
  recognize(recognizer: Recognizer<M>): Analyzer<M>;
  layer(layer: Layer): Analyzer<M>;
  free(): void;
  [Symbol.dispose](): void;
}

/** Static constructors for a modality-specific {@link Analyzer}. */
export const Analyzer: {
  /** An analyzer for the text modality. */
  text(): Analyzer<Text>;
  /** An analyzer for the tabular modality (CSV cells). */
  tabular(): Analyzer<Tabular>;
  /** An analyzer for the image modality (OCR-read pixel text). */
  image(): Analyzer<Image>;
  /** An analyzer for the audio modality (an STT transcript). */
  audio(): Analyzer<Audio>;
};

/**
 * The detect-and-redact pipeline, mirroring the toolkit's `Orchestrator`.
 *
 * A `new Orchestrator()` is seeded with the built-in codecs and label catalog;
 * add a modality's stage with {@link with} (the {@link Analyzer}'s modality
 * selects the codec path and redaction policy), then run it over a blob with
 * {@link redact}. Reusable across calls.
 */
export class Orchestrator {
  constructor();
  with<M extends StageModality>(
    analyzer: Analyzer<M>,
    anonymizer?: Anonymizer<M>,
  ): Orchestrator;
  /**
   * Detect and redact the personal data in `bytes`, whose format the `hint`
   * names (a file extension like `txt`, `csv`, `png`), returning the redacted
   * bytes and every finding.
   *
   * Rejects with an `ElideError` if the format is unknown or the pipeline fails.
   */
  redact(bytes: Uint8Array, hint: string): Promise<RedactionResult>;
  free(): void;
  [Symbol.dispose](): void;
}
