// Shared types for the branded `@nvisy/elide` overlay.
//
// The modality aliases, the phantom brand, and the result shapes are needed by
// the root entry and every building-block subpath, so they live here and are
// re-exported where each is public. Types-only — there is no `_shared.js`.

import type { ByteRange } from "./dist/elide_wasm.js";
export type { ByteRange } from "./dist/elide_wasm.js";

/** The text modality: plain or structured text addressed by byte ranges. */
export type Text = "text";
/** The tabular modality: spreadsheet and CSV content addressed by cell. */
export type Tabular = "tabular";
/** The image modality: raster image content addressed by pixel region. */
export type Image = "image";
/** The audio modality: audio content addressed by time span. */
export type Audio = "audio";
/** The metadata modality: a document's named fields (e.g. EXIF), by key. */
export type Metadata = "metadata";

/** Any modality a finding can be reported in. */
export type Modality = Text | Tabular | Image | Audio | Metadata;

/** The modalities an analyzer stage can detect over. */
export type StageModality = Text | Tabular | Image | Audio;

// The brand sits in a contravariant (parameter) position, so a handle that
// supports *more* modalities is assignable where *fewer* are required: a
// language enricher (`Enricher<StageModality>`) fits every stage, while an OCR
// enricher (`Enricher<Image>`) fits only an image analyzer. The same symbol is
// shared across Enricher/Recognizer/Analyzer, so it must live in one module.
declare const brandSymbol: unique symbol;

/** A phantom modality brand carried by the handle interfaces. */
export interface Branded<M extends StageModality> {
  readonly [brandSymbol]?: (supported: M) => void;
}

/**
 * One detected entity. `range` is the byte span in the decoded text, present
 * only for text-shaped findings; an image or audio finding is located in its own
 * coordinate space, so it is absent there.
 *
 * Narrows the generated `Finding` so `modality` is a {@link Modality}, not
 * `string`.
 */
export interface Finding {
  modality: Modality;
  label: string;
  range: ByteRange | undefined;
  confidence: number;
}

/** The redacted document (re-encoded in the input's format) and every finding. */
export interface RedactionResult {
  redacted: Uint8Array;
  findings: Finding[];
}
