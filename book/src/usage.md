# Usage

Hosted models are billed by the token, the page, or the second of audio.
`Metered` (feature `metrics`) records every call a backend makes: the model,
how long it took, the units it was billed for, how much it found, and
whether it failed. It sends each record to a `UsageSink`.

## Per run

A `UsageCollector` is a sink that adds up the calls sent to it. Give each
run its own collector, and read its total at the end:

```rust
use elide::backend::BackendExt;
use elide::backend::metrics::UsageCollector;
use elide::recognition::ner::NerRecognizer;
# use elide::recognition::ner::backend::MockBackend;

# fn main() {
# let backend = MockBackend::new();
let usage = UsageCollector::new();
let ner = NerRecognizer::new(backend.metered(usage.clone()));

// ... run the analysis, then:
let units = usage.total_units();
let tokens = units.tokens();
# }
```

Because each run owns its collector, a server handling many requests at
once charges each call to the right one, with no shared state.

## Process-wide

For dashboards and alerts, two sinks forward every call to the observability
a deployment already runs:

| Sink | Feature | Sends to |
|---|---|---|
| `TracingSink` | `metrics-tracing` | A `tracing` event per call, on the `elide::usage` target |
| `MetricsSink` | `metrics-recorder` | Counters and histograms through the `metrics` facade, for Prometheus or StatsD |

`Tee` sends to two sinks at once, such as a per-run collector and the
process-wide tracing:

```rust
use elide::backend::BackendExt;
use elide::backend::metrics::{Tee, TracingSink, UsageCollector};
# use elide::recognition::ner::backend::MockBackend;

# fn main() {
# let backend = MockBackend::new();
let usage = UsageCollector::new();
let backend = backend.metered(Tee::new(usage.clone(), TracingSink::new()));
# }
```

Metrics are labelled by model, version, and outcome only. Those are a small
set, fixed by configuration; per-request identifiers would make the number
of series explode, which is why per-run usage belongs to a collector instead.
