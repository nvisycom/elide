// The model callbacks a `Backend` wraps, and what each is called with and
// returns: the contract between a recognizer or enricher and the JS code that
// runs its model. Re-exported by `@nvisy/elide/analyzer`.
// Types-only — there is no `_callbacks.js`.

// For the doc links only.
import type { Enricher, Recognizer } from "./analyzer.js";

/**
 * One span a {@link NerCallback} returns: a labeled byte range in the request
 * text with the model's confidence.
 */
export interface NerSpan {
  /** The canonical label id (e.g. `person_name`, `organization_name`). */
  label: string;
  /** Start byte offset of the span in the request text. */
  start: number;
  /** End byte offset (exclusive). */
  end: number;
  /** Model confidence in `[0, 1]`. */
  score: number;
}

/**
 * The NER inference callback a {@link Recognizer.ner} backend wraps. It receives the text
 * to scan and the label ids the recognizer wants, and returns the spans its
 * model found — a promise, or the array directly.
 */
export type NerCallback = (
  text: string,
  labels: string[],
) => NerSpan[] | Promise<NerSpan[]>;

/**
 * One text entity an {@link LlmCallback} returns: the literal `value` and its
 * `label`, with the surrounding `context` that places it. The recognizer finds
 * the context in the text and the value within it, so no offsets are needed; a
 * candidate whose context is missing, not found, or found more than once is
 * dropped.
 */
export interface TextCandidate {
  /** The literal text to flag, as it appears in the source. */
  value: string;
  /** The label id naming the entity's type (e.g. `person_name`). */
  label: string;
  /** A short description of the entity. */
  description?: string;
  /** The model's confidence in `[0, 1]`; `0.5` when unset. */
  confidence?: number;
  /** A short run of text around `value` that occurs once in the source. */
  context: string;
  /** An id shared by every mention of one real-world entity in the call. */
  coreference?: string;
}

/**
 * One image region an {@link LlmCallback} returns: its box, as fractions of the
 * image's width and height, and its `label`.
 */
export interface ImageCandidate {
  /** Left edge, in `[0, 1]` of the width. */
  x_min: number;
  /** Top edge, in `[0, 1]` of the height. */
  y_min: number;
  /** Right edge, in `[0, 1]` of the width. */
  x_max: number;
  /** Bottom edge, in `[0, 1]` of the height. */
  y_max: number;
  /** The label id naming the region's type (e.g. `face`). */
  label: string;
  /** A short description of the region. */
  description?: string;
  /** The model's confidence in `[0, 1]`; `0.5` when unset. */
  confidence?: number;
  /** An id shared by every mention of one real-world entity in the call. */
  coreference?: string;
}

/** What an {@link LlmCallback} is called with besides the prompt. */
export interface LlmRequest {
  /** The stage the call is for. */
  modality: "text" | "image";
  /**
   * The JSON Schema of the response's `{ entities }`, for a model's structured
   * output mode.
   */
  schema: object;
  /** The call's language (a BCP-47 tag), when known. */
  language?: string;
  /** On an image stage, the image as PNG bytes. */
  image?: Uint8Array;
}

/** What an {@link LlmCallback} returns, mirroring the toolkit's `LlmResponse`. */
export interface LlmResponse {
  /** The entities the model found: text or image candidates, by stage. */
  entities: TextCandidate[] | ImageCandidate[];
  /** The model that answered, for the audit trail. */
  model?: string;
}

/**
 * The model call {@link Recognizer.llm} wraps. It receives the rendered prompt
 * and the {@link LlmRequest}, calls the model, and returns its response — a
 * promise, or the response directly.
 */
export type LlmCallback = (
  prompt: string,
  request: LlmRequest,
) => LlmResponse | Promise<LlmResponse>;

/**
 * One language a {@link LanguageCallback} detects: a BCP-47 tag, the
 * confidence, and the byte span it covers when not the whole text.
 */
export interface LanguageDetection {
  /** The language, a BCP-47 tag such as `de`. */
  language: string;
  /** The detector's confidence in `[0, 1]`. */
  confidence: number;
  /** Start byte offset of the span it covers; set with `end`, or neither. */
  start?: number;
  /** End byte offset (exclusive) of the span it covers. */
  end?: number;
}

/**
 * The language detection callback an {@link Enricher.language} backend wraps:
 * the browser's own `LanguageDetector`, a JS library, or an endpoint. It
 * receives the text and the languages asserted with
 * `Orchestrator.withLanguages` to choose among (empty for any), and returns the
 * languages detected — a promise, or the array directly. A detection outside
 * the candidates is dropped; with one asserted language it is not called.
 */
export type LanguageCallback = (
  text: string,
  candidates: string[],
) => LanguageDetection[] | Promise<LanguageDetection[]>;

/**
 * One recognized text region an {@link OcrCallback} returns: the text and its
 * box (top-left corner and size, in pixels), with an optional confidence.
 *
 * A region is the atom OCR engines emit — typically a word, but a coarser run
 * (a whole line) works too. A recognizer scans the regions' text joined by
 * spaces; a match redacts the boxes of the regions it covers.
 */
export interface OcrRegion {
  /** The recognized text of the region. */
  text: string;
  /** Left edge of the box, in pixels. */
  x: number;
  /** Top edge of the box, in pixels. */
  y: number;
  /** Box width, in pixels. */
  width: number;
  /** Box height, in pixels. */
  height: number;
  /** Recognition confidence in `[0, 1]`, when the engine reports it. */
  confidence?: number;
}

/**
 * The OCR inference callback an {@link Enricher.ocr} backend wraps. It receives
 * the encoded image bytes and the call's language (a BCP-47 tag) when known,
 * and returns the recognized text regions — a promise, or the array directly.
 */
export type OcrCallback = (
  image: Uint8Array,
  language?: string,
) => OcrRegion[] | Promise<OcrRegion[]>;

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
 * The STT inference callback an {@link Enricher.stt} backend wraps. It receives
 * the encoded audio bytes and the call's language (a BCP-47 tag) when known,
 * and returns the transcript segments — a promise, or the array directly.
 */
export type SttCallback = (
  audio: Uint8Array,
  language?: string,
) => SttSegment[] | Promise<SttSegment[]>;
