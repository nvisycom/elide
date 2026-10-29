# Reconciling findings

Recognizers run independently, so their findings overlap. Two recognizers
report the same email. One reports a span as a person, another as an
organization. A postal code sits inside an address. *Layers* turn these into
one consistent set, in the order they are added:

```rust
use elide::prelude::*;

let analyzer = Analyzer::<Text>::new()
    .with_layer(ReconcileLayer::same_label(Merging::max()))
    .with_layer(ReconcileLayer::cross_label(Structural::default()))
    .with_layer(FilterLayer::new().with_threshold(ConfidenceThreshold::BASELINE));
```

## Calibrating

Recognizers score on different scales: one model's 0.8 is not another's.
`CalibrateLayer` maps each recognizer's scores onto a common scale with a
`CalibrationMap`, so later layers compare like with like. Put it first.

## Same label

`ReconcileLayer::same_label` fuses findings of one label that cover the
same content into one entity. Its strategy decides the fused confidence:
`Merging::max()` keeps the highest. The fused entity's audit trail keeps
every recognizer's finding.

## Different labels

`ReconcileLayer::cross_label` resolves findings of different labels that
overlap. `Structural`, the default, reads how the two spans relate:

- **Nested**: a postal code inside an address is a real hierarchy, so both
  are kept, unless the inner one is much weaker.
- **Touching**: two findings that merely overlap at the edges are both kept.
- **Coincident**: two labels on substantially the same span are a conflict.
  By default a tiebreaker picks the winner; `Structural::default().reviewing()`
  keeps both and marks them contested instead, for a person to decide during
  [review](review.md).

## Filtering

`FilterLayer` drops findings below a confidence threshold, or outside a set
of allowed labels. `ConfidenceThreshold::BASELINE` is a sensible default.

Weak findings can also be kept and handled at redaction time instead: a
[rule](rules.md) can leave entities below a threshold untouched while still
recording them.
