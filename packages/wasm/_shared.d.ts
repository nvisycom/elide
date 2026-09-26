// Shared types for the branded `@nvisy/elide` overlay.
//
// The modality aliases, the phantom brand, and the wasm-bindgen output types are
// needed by the root entry and every subpath, so they are re-exported here.
// Types-only — there is no `_shared.js`.

import type { Modality } from "./dist/elide_wasm.js";
export type { Modality, Location, Entity, Report } from "./dist/elide_wasm.js";

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

/** The modalities an analyzer stage can detect over (every modality but metadata). */
export type StageModality = Exclude<Modality, Metadata>;

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
