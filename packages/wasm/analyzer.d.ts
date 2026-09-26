// `@nvisy/elide/analyzer` — the detect side: an `Analyzer` stage and the
// recognizers, enrichers, and reconcile/filter layers it folds in.

import type {
  Branded,
  StageModality,
  Text,
  Tabular,
  Image,
  Audio,
} from "./_shared.js";
import type {
  LanguageSet,
  PatternRecognizerConfig,
  SameLabelScoring,
  CrossLabelConfig,
} from "./dist/elide_wasm.js";

export type {
  LanguagePreset,
  LanguageSet,
  PatternRecognizerConfig,
  SameLabelScoring,
  ExclusiveTiebreaker,
  CrossLabelConfig,
} from "./dist/elide_wasm.js";

/**
 * One span a {@link NerCallback} returns: a labeled byte range in the request
 * text with the model's confidence.
 */
export interface NerSpan {
  /** The canonical label id (e.g. `person`, `organization`). */
  label: string;
  /** Start byte offset of the span in the request text. */
  start: number;
  /** End byte offset (exclusive). */
  end: number;
  /** Model confidence in `[0, 1]`. */
  score: number;
}

/**
 * The NER inference callback {@link Recognizer.ner} wraps. It receives the text
 * to scan and the label ids the recognizer wants, and returns the spans its
 * model found — a promise, or the array directly.
 */
export type NerCallback = (
  text: string,
  labels: string[],
) => NerSpan[] | Promise<NerSpan[]>;

/** A pixel-space bounding box: its top-left corner and size, in pixels. */
export interface OcrBox {
  /** Left edge, in pixels. */
  x: number;
  /** Top edge, in pixels. */
  y: number;
  /** Box width, in pixels. */
  width: number;
  /** Box height, in pixels. */
  height: number;
}

/** One word within an {@link OcrBlock}, with its own box and confidence. */
export interface OcrWord extends OcrBox {
  /** The word text, as it appears in the block text. */
  text: string;
  /** Recognition confidence in `[0, 1]`, when the engine reports it. */
  confidence?: number;
}

/**
 * One recognized text block an {@link OcrCallback} returns, in image-pixel
 * coordinates, optionally split into per-word boxes.
 *
 * A recognizer scans the block `text`; redaction covers the matched `words`'
 * boxes when present, and the block box otherwise. Emitting words (as browser
 * OCR engines do) redacts the matched word rather than the whole block.
 */
export interface OcrBlock extends OcrBox {
  /** The recognized text of the block. */
  text: string;
  /**
   * The block's words, each with its own box. Omit (or leave empty) to report
   * only the block; redaction then falls back to the block box.
   */
  words?: OcrWord[];
}

/**
 * The OCR inference callback {@link Enricher.ocr} wraps. It receives the encoded
 * image bytes and returns the recognized text blocks — a promise, or the array
 * directly.
 */
export type OcrCallback = (
  image: Uint8Array,
) => OcrBlock[] | Promise<OcrBlock[]>;

/** One transcript segment an {@link SttCallback} returns, with its time span. */
export interface SttSegment {
  /** The transcribed text of the segment. */
  text: string;
  /** Segment start, in milliseconds. */
  startMs: number;
  /** Segment end, in milliseconds. */
  endMs: number;
}

/**
 * The STT inference callback {@link Enricher.stt} wraps. It receives the encoded
 * audio bytes and returns the transcript segments — a promise, or the array
 * directly.
 */
export type SttCallback = (
  audio: Uint8Array,
) => SttSegment[] | Promise<SttSegment[]>;

/**
 * An opaque recognizer handle, branded with the modalities `M` it applies to.
 *
 * Built by {@link Recognizer.pattern} or {@link Recognizer.ner} (both
 * text-shaped, so every stage) and consumed by {@link Analyzer.recognize}. The
 * contravariant brand lets a recognizer that supports more modalities fit where
 * fewer are required.
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
  ner(callback: NerCallback): Recognizer<StageModality>;
};

/**
 * An opaque enricher handle, branded with the modalities `M` it applies to.
 *
 * Built by {@link Enricher.language} (every modality), {@link Enricher.ocr}
 * ({@link Image}), or {@link Enricher.stt} ({@link Audio}), and consumed by
 * {@link Analyzer.enrich}. Passing one to a stage it does not support is a type
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
  ocr(callback: OcrCallback): Enricher<Image>;
  /** An STT enricher from a JS callback; applies to the audio modality. */
  stt(callback: SttCallback): Enricher<Audio>;
};

/**
 * A detection or reconciliation layer, built by {@link Layer.filter},
 * {@link Layer.reconcileSameLabel}, or {@link Layer.reconcileCrossLabel}.
 *
 * A layer applies to any modality, so — unlike the enricher and recognizer
 * handles — it is unbranded and fits any {@link Analyzer}.
 */
export interface Layer {
  free(): void;
  [Symbol.dispose](): void;
}

/** Static constructors for a {@link Layer}. */
export const Layer: {
  /** A filter layer that drops findings below `threshold` (in `[0, 1]`). */
  filter(threshold: number): Layer;
  /**
   * A same-label reconcile layer that merges overlapping findings of the same
   * label with the given `scoring`.
   */
  reconcileSameLabel(scoring: SameLabelScoring): Layer;
  /**
   * A cross-label reconcile layer that arbitrates overlapping findings of
   * different labels per `config`.
   */
  reconcileCrossLabel(config: CrossLabelConfig): Layer;
};

/**
 * The detect side of a modality's stage, branded with its modality `M`,
 * mirroring the toolkit's `Analyzer` builder.
 *
 * Built with {@link Analyzer.text}/{@link Analyzer.tabular}/{@link Analyzer.image}/
 * {@link Analyzer.audio}, then extended with {@link enrich}, {@link recognize},
 * and {@link layer}. The `M` brand makes a mismatched handle — an OCR enricher on
 * a text analyzer — a compile error. Hand it to an `Orchestrator`'s `with`.
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
