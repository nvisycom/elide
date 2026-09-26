// `@nvisy/elide/anonymizer` — the redaction side of a stage.

import type {
  Branded,
  StageModality,
  Text,
  Tabular,
  Image,
  Audio,
} from "./_shared.js";

export { Label } from "./label.js";

/**
 * The tuning for {@link Operator.mask}. All fields are optional: an unset field
 * takes its default — mask character `*`, and no verbatim head or tail.
 *
 * (Declared here rather than re-exported from the generated types, which model
 * the optional fields as `T | undefined` — present but nullable — instead of
 * genuinely optional.)
 */
export interface MaskConfig {
  /** Leading characters to keep verbatim (default `0`). */
  keepPrefix?: number;
  /** Trailing characters to keep verbatim (default `0`). */
  keepSuffix?: number;
  /** The character each masked position is replaced with (default `*`). */
  maskChar?: string;
}

/**
 * A redaction operator, branded with the modalities `M` it applies to.
 *
 * Built by {@link Operator.replace}/{@link Operator.mask} (text and tabular),
 * {@link Operator.blackbox} ({@link Image}), {@link Operator.silence}
 * ({@link Audio}), or {@link Operator.erase}/{@link Operator.keep} (broad), and
 * folded into a {@link Rule}. Using one where its modality does not apply is a
 * type error.
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
  /** Remove the matched value entirely (text, tabular, audio). */
  erase(): Operator<Text | Tabular | Audio>;
  /** Leave the matched value untouched (text, tabular, image). */
  keep(): Operator<Text | Tabular | Image>;
  /** Paint a solid box over the matched region (image). */
  blackbox(): Operator<Image>;
  /** Silence the matched time span (audio). */
  silence(): Operator<Audio>;
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
