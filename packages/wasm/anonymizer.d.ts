// `@nvisy/elide/anonymizer` — the redaction side of a stage.

import type {
  Branded,
  StageModality,
  Text,
  Tabular,
  Image,
  Audio,
} from "./_shared.js";

import type {
  BeepConfig,
  ClampConfig,
  FakeConfig,
  GeneralizeDateConfig,
  HashConfig,
  HmacConfig,
  MaskConfig,
  TruncateConfig,
} from "./dist/elide_wasm.js";

export { Label } from "./label.js";
export type {
  BeepConfig,
  ClampBound,
  ClampConfig,
  DateGranularity,
  DateStyle,
  FakeConfig,
  GeneralizeDateConfig,
  HashConfig,
  HmacConfig,
  MaskConfig,
  Sha2Algorithm,
  TruncateConfig,
  Waveform,
} from "./dist/elide_wasm.js";

/**
 * A redaction operator, branded with the modalities `M` it applies to.
 *
 * Built by a text operator such as {@link Operator.replace} or
 * {@link Operator.hmac} (text and tabular), a tabular, image or audio operator
 * such as {@link Operator.dropRow}, {@link Operator.blur} or
 * {@link Operator.beep}, or {@link Operator.cover} (every modality), and folded
 * into a {@link Rule}. Using one where its modality does not apply is a type
 * error.
 */
export interface Operator<M extends StageModality = StageModality>
  extends Branded<M> {
  free(): void;
  [Symbol.dispose](): void;
}

/** Static constructors for an {@link Operator}. */
export const Operator: {
  /** Substitute a fixed `template` for the matched value (text, tabular). */
  replace(template: string): Operator<Text | Tabular>;
  /** Mask the matched value per `config` (text, tabular). */
  mask(config: MaskConfig): Operator<Text | Tabular>;
  /**
   * Keep the matched value's head and tail per `config` and drop the middle
   * (text, tabular).
   */
  truncate(config: TruncateConfig): Operator<Text | Tabular>;
  /**
   * Replace a number at or past a bound by the bound's label, like an age of
   * `94` by `90+` (text, tabular). A number within the bounds is left as it
   * is; a value that is not a number is removed. Throws if `config` sets no bound, a bound
   * that is not finite, or a floor not below its ceiling.
   */
  clamp(config: ClampConfig): Operator<Text | Tabular>;
  /**
   * Reduce a date to a coarser granularity, like `1987-03-14` to `1987`
   * (text, tabular). A value that is not a date in the configured style is
   * removed. Without `config`, keeps the year of an ISO 8601 date.
   */
  generalizeDate(config?: GeneralizeDateConfig): Operator<Text | Tabular>;
  /**
   * Replace the matched value by a realistic fake of its label (text,
   * tabular); a label it cannot fake gets `fallback`, which must be a text
   * operator.
   */
  fake(
    fallback: Operator<Text | Tabular>,
    config?: FakeConfig,
  ): Operator<Text | Tabular>;
  /**
   * Replace the matched value by its SHA-2 hash in hex (text, tabular). A
   * short value can be recovered from its hash by trying every candidate;
   * prefer {@link Operator.hmac}.
   */
  hash(config?: HashConfig): Operator<Text | Tabular>;
  /**
   * Replace the matched value by its HMAC under the secret `key`, in hex (text,
   * tabular). Throws on an empty key.
   */
  hmac(key: Uint8Array, config?: HmacConfig): Operator<Text | Tabular>;
  /**
   * Replace the matched value by its AES-256-GCM encryption under `key`,
   * base64-encoded (text, tabular). Throws unless the key is 32 bytes.
   */
  encrypt(key: Uint8Array): Operator<Text | Tabular>;
  /** Remove the matched value entirely (text, tabular, audio). */
  remove(): Operator<Text | Tabular | Audio>;
  /**
   * Hide the matched value in place, keeping its extent: a run of `█` in text,
   * a black box in an image, silence in audio.
   */
  cover(): Operator<Text | Tabular | Image | Audio>;
  /** Leave the matched value untouched (text, tabular, image). */
  keep(): Operator<Text | Tabular | Image>;
  /** Drop the whole row of the matched cell (tabular). */
  dropRow(): Operator<Tabular>;
  /** Drop the whole column of the matched cell (tabular). */
  dropColumn(): Operator<Tabular>;
  /**
   * Blur the matched region with a Gaussian of `sigma` pixels (image). Throws
   * on a `sigma` below `1`.
   */
  blur(sigma: number): Operator<Image>;
  /**
   * Pixelate the matched region in blocks of `blockSize` pixels (image).
   * Throws on a block size below `2`, which leaves the region as it is.
   */
  pixelate(blockSize: number): Operator<Image>;
  /** Replace the matched span by a tone, keeping the timeline (audio). */
  beep(config?: BeepConfig): Operator<Audio>;
};

/**
 * A redaction rule, branded with the modalities `M` its operator applies to.
 *
 * Built by {@link Rule.label} (act on one label) or {@link Rule.fallback} (act on
 * anything unmatched) and folded into an {@link Anonymizer}. The `M` flows from
 * the operator, so a rule built from an image operator does not fit a text
 * anonymizer.
 */
export interface Rule<M extends StageModality = StageModality>
  extends Branded<M> {
  free(): void;
  [Symbol.dispose](): void;
}

/** Static constructors for a {@link Rule}. */
export const Rule: {
  /**
   * A rule applying `operator` to every entity of `label` (a label id such as
   * `"email_address"`, or a {@link Label} constant).
   */
  label<M extends StageModality>(label: string, operator: Operator<M>): Rule<M>;
  /** A catch-all rule applying `operator` to every unmatched entity. */
  fallback<M extends StageModality>(operator: Operator<M>): Rule<M>;
};

/**
 * The redaction side of a modality's stage, branded with its modality `M`,
 * mirroring the toolkit's `Anonymizer` builder.
 *
 * Built with {@link Anonymizer.text}/{@link Anonymizer.tabular}/
 * {@link Anonymizer.image}/{@link Anonymizer.audio}, then extended with
 * {@link rule}. With no rules, the stage uses a readable default policy. Hand it
 * to `Orchestrator.with` alongside the matching analyzer.
 */
export interface Anonymizer<M extends StageModality = StageModality>
  extends Branded<M> {
  rule(rule: Rule<M>): Anonymizer<M>;
  free(): void;
  [Symbol.dispose](): void;
}

/** Static constructors for a modality-specific {@link Anonymizer}. */
export const Anonymizer: {
  /** An anonymizer for the text modality. */
  text(): Anonymizer<Text>;
  /** An anonymizer for the tabular modality (CSV cells). */
  tabular(): Anonymizer<Tabular>;
  /** An anonymizer for the image modality (redacted pixel regions). */
  image(): Anonymizer<Image>;
  /** An anonymizer for the audio modality (redacted time spans). */
  audio(): Anonymizer<Audio>;
};
