# Rules

An `Anonymizer<M>` hides the entities an analyzer found. It holds a list of
*rules*, each pairing a selector with an [operator](operators.md). For each
entity, the first rule that matches picks the operator:

```rust
use elide::prelude::operators::*;
use elide::prelude::*;

let anonymizer = Anonymizer::<Text>::new()
    // Leave weak findings untouched, but recorded.
    .with(Rule::predicate(
        |cx| !ConfidenceThreshold::BASELINE.passes(cx.entity.confidence),
        Keep,
    ))
    .with(Rule::label(builtins::EMAIL_ADDRESS.to_ref(), Replace::new("[EMAIL]")))
    .with(Rule::label(builtins::PAYMENT_CARD.to_ref(), Mask::stars().with_keep_suffix(4)))
    .with(Rule::fallback(Erase));
```

| Rule | Matches |
|---|---|
| `Rule::label(label, op)` | Entities with that label |
| `Rule::tag(tag, op)` | Entities whose label carries that tag in the catalog, such as `"pii"` |
| `Rule::predicate(f, op)` | Entities for which `f` returns true; it sees the entity, the catalog, and the scope |
| `Rule::fallback(op)` | Every entity; put it last |

An entity no rule matches is left as it is. A `Rule::fallback` makes sure
nothing found slips through.

`Rule::tag` reads the catalog, so the anonymizer needs one:

```rust
use elide::prelude::operators::*;
use elide::prelude::*;

let anonymizer = Anonymizer::<Text>::new()
    .with_catalog(LabelCatalog::with_builtins())
    .with(Rule::tag("pii", Replace::new("[REDACTED]")));
```

## Why a rule fired

`.because(…)` attaches a reason, such as the policy clause that requires it.
The reason is recorded on the audit trail of every entity the rule hides:

```rust
use elide::entity::audit::Attribution;
use elide::prelude::operators::*;
use elide::prelude::*;

let rule = Rule::<Text>::label(builtins::IBAN.to_ref(), Erase)
    .because(Attribution::freeform("gdpr-art-17"));
```

## Applying

`anonymize` picks an operator for each entity and applies it in one step.
To look at the picks first, call `pick`, which records each choice on its
entity, and later `redact`, which applies them.
