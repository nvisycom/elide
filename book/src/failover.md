# Failover

Hosted models have outages, quotas, and timeouts. When one fails, an
analysis fails with it, because a recognizer whose model errored has not
vouched that the content is clean. `FallbackRouted` (feature `routing`)
keeps the analysis going: it calls a primary backend, and when that fails,
routes the same request to a fallback.

```rust
use elide::backend::BackendExt;
use elide::prelude::*;
use elide::recognition::ner::NerRecognizer;
use elide::recognition::ner::backend::NerBackendExt;
# use elide::recognition::ner::backend::MockBackend;

# fn main() {
# let (hosted, local) = (MockBackend::new(), MockBackend::new());
// A hosted model, and a smaller local one for when it is unavailable.
let backend = hosted.fallback_routed(local.label_scoped([builtins::PERSON_NAME.to_ref()]));

let ner = NerRecognizer::new(backend);
# }
```

The two can be different backend types, as long as they take the same
request and return the same response. To fall back more than once, chain
them: `a.fallback_routed(b.fallback_routed(c))`. The analysis fails only
when every backend has failed.

## Which errors fail over

By default, a call fails over on the errors another backend could avoid:

| Error kind | For example |
|---|---|
| `Transport` | The network failed or the call timed out |
| `Provider` | An outage, an exhausted quota, an overloaded model |
| `CapabilityUnavailable` | A language or format this backend does not serve |

Any other error is returned as it is. Malformed input or a misconfiguration
would fail the same way on another backend, or failing over would hide
something to fix. `when` replaces the condition:

```rust
use elide::backend::BackendExt;
use elide::prelude::*;
# use elide::recognition::ner::backend::MockBackend;

# fn main() {
# let (primary, fallback) = (MockBackend::new(), MockBackend::new());
// Only fail over when the primary is unreachable.
let backend = primary
    .fallback_routed(fallback)
    .when(|error| error.kind() == ErrorKind::Transport);
# }
```

A batch fails over whole, since a batch fails whole.

## Knowing when it happens

A failover is silent to the analysis, so watch for it in usage. Meter the
primary and the fallback, not the router: each call is then charged to the
model that served it, and the primary's failed calls are recorded even when
the fallback recovered from them. See [Usage and cost](usage.md).

```rust
use elide::backend::BackendExt;
use elide::backend::metrics::UsageCollector;
# use elide::recognition::ner::backend::MockBackend;

# fn main() {
# let (hosted, local) = (MockBackend::new(), MockBackend::new());
let usage = UsageCollector::new();
let backend = hosted
    .metered(usage.clone())
    .fallback_routed(local.metered(usage.clone()));
# }
```
