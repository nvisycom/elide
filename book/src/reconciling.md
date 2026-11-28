# Reconciling findings

Recognizers run independently, so their findings overlap. Two recognizers
report the same email. One reports a span as a person, another as an
organization. A postal code sits inside an address. *Layers* turn these into
one consistent set, in the order they are added:

```rust
use elide::prelude::*;

let analyzer = Analyzer::<Text>::new()
    .with_layer(FuseLayer::max())
    .with_layer(ReconcileLayer::structural())
    .with_layer(ThresholdLayer::default());
```

## Calibrating

Recognizers score on different scales: one model's 0.8 is not another's.
Calibration is not a layer: wrap a recognizer in a `Calibrated` and
its scores are scaled as it reports them, before any layer runs, so the layers
compare like with like. See [recognizer decorators](recognizer-decorators.md#calibration).

## Same label

`FuseLayer` fuses findings of one label that cover the same content into one
entity. Its scoring decides the fused confidence: `FuseLayer::max()` keeps the
highest, `FuseLayer::noisy_or()` accumulates agreeing findings' evidence. The
fused entity's audit trail keeps every recognizer's finding, so fusion drops
nothing.

## Different labels

`ReconcileLayer` resolves findings of different labels that overlap.
`Structural`, the default (`ReconcileLayer::structural()`), reads how the
two spans relate:

- **Nested**: a postal code inside an address is a real hierarchy, so both
  are kept, unless the inner one is much weaker.
- **Touching**: two findings that merely overlap at the edges are both kept.
- **Coincident**: two labels on substantially the same span are a conflict.
  By default a tiebreaker picks the winner; `Structural::default().reviewing()`
  keeps both and marks them contested instead, for a person to decide during
  [review](review.md).

## Filtering

`ThresholdLayer` drops findings below a confidence threshold. Its default,
`ConfidenceThreshold::BASELINE`, is a sensible starting point; pass another
with `ThresholdLayer::new`. Which labels are detected is the request's
catalog, not a layer: after every layer, the analyzer drops the labels the
catalog does not declare. It runs last so an out-of-catalog finding can still
win an overlap against a weak in-catalog one first.

Weak findings can also be kept and handled at redaction time instead: a
[rule](rules.md) can leave entities below a threshold untouched while still
recording them.

## Why a finding is missing

An analysis keeps everything it drops, beside its findings. A dropped entity's
audit trail holds a `Dropped` event: the component that dropped it (a layer,
or the catalog filter) and, when it lost an overlap, the id of the entity that
beat it. That entity may itself be dropped, so a chain reads in full:

```rust
use elide::detection::Analysis;
use elide::modality::text::Text;

fn explain(analysis: &Analysis<Text>) {
    for entity in &analysis.entities {
        let Some(dropped) = entity.audit().dropped() else {
            continue;
        };
        let label = entity.label().as_str();
        match dropped.superseded_by {
            Some(winner) => println!("{label} lost to {winner} ({})", dropped.by),
            None => println!("{label} dropped by {}", dropped.by),
        }
    }
}
```

The winner records each loser too: its `Resolved` event names the loser's id
and the reconciler that decided.

A dropped entity is suppressed: redaction leaves it alone, and it travels in
the report like any other, so a reviewer can read why it is missing and
[confirm](review.md) it, which puts it back in front of redaction. Where the
candidates a noisy recognizer drops would swamp the report,
`Analyzer::discard_dropped` keeps the analysis to its findings.

A custom layer derives `Component`, like a recognizer, and implements
`Layer<M>`: for each entity it calls `keep`, `reject`, or `supersede` on its
`LayerOutput`. The analyzer names whatever it drops with its name and version.
