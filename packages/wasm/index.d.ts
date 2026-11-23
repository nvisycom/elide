// `@nvisy/elide` — the pipeline entry.
//
// The `Orchestrator` and the shared vocabulary (modality types, entities, the
// error class). The detect and redact stages live under the `/analyzer` and
// `/anonymizer` subpaths.

import type { StageModality } from "./_shared.js";
import type { Report } from "./_shared.js";
import type { Analyzer } from "./analyzer.js";

export { ElideError, ElideErrorKind } from "./dist/elide_wasm.js";
export type {
  Text,
  Tabular,
  Image,
  Audio,
  Metadata,
  Modality,
  StageModality,
  Entity,
  Location,
  Report,
} from "./_shared.js";

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
   * names (a file extension like `txt`, `docx`, `pdf`), returning the redacted
   * bytes and every entity. Without a hint the format is recognized by its
   * leading bytes, which only the binary formats carry.
   *
   * Rejects with an `ElideError` if the format is unknown or the pipeline fails.
   */
  redact(bytes: Uint8Array, hint?: string): Promise<Report>;
  free(): void;
  [Symbol.dispose](): void;
}
