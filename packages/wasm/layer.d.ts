// `@nvisy/elide/layer` — post-detection reconcile/filter layers.

import type {
  SameLabelScoring,
  CrossLabelConfig,
} from "./dist/elide_wasm.js";

export type {
  SameLabelScoring,
  ExclusiveTiebreaker,
  CrossLabelConfig,
} from "./dist/elide_wasm.js";

/**
 * A detection or reconciliation layer, built by {@link Layer.filter},
 * {@link Layer.reconcileSameLabel}, or {@link Layer.reconcileCrossLabel}.
 *
 * A layer applies to any modality, so — unlike the enricher and recognizer
 * handles — it is unbranded and fits any analyzer.
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
