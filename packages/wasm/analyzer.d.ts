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
  PatternRecognizerConfig,
  RateConfig,
  SameLabelScoring,
  CrossLabelConfig,
} from "./dist/elide_wasm.js";
import type {
  NerCallback,
  LlmCallback,
  LanguageCallback,
  OcrCallback,
  SttCallback,
} from "./_callbacks.js";

export type * from "./_callbacks.js";
export type {
  PatternRecognizerConfig,
  RateConfig,
  SameLabelScoring,
  ExclusiveTiebreaker,
  CrossLabelConfig,
} from "./dist/elide_wasm.js";

// Carries a backend's callback type, so a NER backend does not fit where an LLM
// backend is required. Types-only.
declare const callbackSymbol: unique symbol;

/** Any model callback a {@link Backend} can wrap. */
export type BackendCallback =
  | NerCallback
  | LlmCallback
  | LanguageCallback
  | OcrCallback
  | SttCallback;

/**
 * A model backend whose inference is a JS callback of type `C`, with the limits
 * and routes that bound how it is called.
 *
 * Built by {@link Backend.callback} and handed to {@link Recognizer.ner},
 * {@link Recognizer.llm}, {@link Enricher.language}, {@link Enricher.ocr} or
 * {@link Enricher.stt}, which
 * call it with their own arguments. The decorators are the toolkit's
 * `BackendExt` ones, by the same names. Each method consumes the handle and
 * returns the wrapped one, so a stack reads innermost first:
 *
 * ```ts
 * const backend = Backend.callback<NerCallback>(callModel)
 *   .rateLimited({ count: 60, intervalMs: 60_000 })
 *   .concurrencyLimited(4)
 *   .fallbackRouted(Backend.callback<NerCallback>(callSmallerModel));
 * ```
 */
export interface Backend<C extends BackendCallback = BackendCallback> {
  readonly [callbackSymbol]?: C;
  /**
   * Admit calls at `rate` (`count` per `intervalMs`, after an initial `burst`
   * that defaults to `count`), waiting when it is used up. Throws on a zero
   * count, interval or burst.
   */
  rateLimited(rate: RateConfig): Backend<C>;
  /** Run at most `max` calls at once. Throws on zero. */
  concurrencyLimited(max: number): Backend<C>;
  /**
   * Send a call whose callback throws or rejects to `fallback` instead. A
   * response that cannot be read is not retried.
   */
  fallbackRouted(fallback: Backend<C>): Backend<C>;
  /**
   * Route requests by language: this backend serves every language without a
   * {@link route}, and requests whose language is unknown.
   */
  languageRouted(): Backend<C>;
  /**
   * Send requests in `language` (a BCP-47 tag such as `de`) to `backend`.
   * Throws on an invalid tag, or on a backend that is not
   * {@link languageRouted}.
   */
  route(language: string, backend: Backend<C>): Backend<C>;
  free(): void;
  [Symbol.dispose](): void;
}

/** Static constructors for a {@link Backend}. */
export const Backend: {
  /**
   * A backend whose inference is `callback`. Name its type to have the
   * callback's parameters typed: `Backend.callback<NerCallback>(...)`.
   */
  callback<C extends BackendCallback>(callback: C): Backend<C>;
};

/**
 * An opaque recognizer handle, branded with the modalities `M` it applies to.
 *
 * Built by {@link Recognizer.pattern} or {@link Recognizer.ner} (both
 * text-shaped, so every stage) or {@link Recognizer.llm} (text and image), and
 * consumed by {@link Analyzer.recognize}. The contravariant brand lets a
 * recognizer that supports more modalities fit where fewer are required.
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
  /** A NER recognizer over a JS-callback backend; applies to every text-shaped modality. */
  ner(backend: Backend<NerCallback>): Recognizer<StageModality>;
  /**
   * An LLM recognizer over a JS-callback backend; applies to the text and
   * image modalities. The prompt is the built-in wording, or `template`, a
   * Jinja2 template over `text` (text stages), `language`, `target_labels` and
   * `tags`. Throws if the template does not compile.
   */
  llm(backend: Backend<LlmCallback>, template?: string): Recognizer<Text | Image>;
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
  /**
   * A language-detection enricher over a JS-callback backend; applies to every
   * modality. Throws on a `languageRouted` backend, which has no language to
   * route a detection by.
   */
  language(backend: Backend<LanguageCallback>): Enricher<StageModality>;
  /** An OCR enricher over a JS-callback backend; applies to the image modality. */
  ocr(backend: Backend<OcrCallback>): Enricher<Image>;
  /** An STT enricher over a JS-callback backend; applies to the audio modality. */
  stt(backend: Backend<SttCallback>): Enricher<Audio>;
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
