# elide-redaction

[![Build](https://img.shields.io/github/actions/workflow/status/nvisycom/elide/build.yml?branch=main&label=build%20%26%20test&style=flat-square)](https://github.com/nvisycom/elide/actions/workflows/build.yml)

The redaction engine for PII/PHI: the `Anonymizer` and `Deanonymizer` that
select and apply operators.

## Overview

Once entities are detected, they have to be hidden — and the *how* is a policy
decision: mask a phone number, replace an email, encrypt a record number so it
can be recovered, drop a whole table row. This crate is the "hide" engine: it
*selects* an operator per entity and *applies* it. The operators themselves —
the strategies — live in [`elide-operator`], so the engine can be depended on
without the operator library, and vice versa.

`Anonymizer` is the redaction counterpart to the detection `Analyzer`. It holds
an ordered list of selection `Rule`s (bind an operator to a label, a tag, a
predicate over a `MatchContext`, or a catch-all fallback) and two paths:

- `select` runs the rules and hands back a reviewable `Selection` per redaction
  (which operator won, why, over which entities) without reading any data — the
  decision phase, inspectable and editable before anything is applied. Take a
  `Selection::view` for a serializable `SelectionView`.
- `anonymize` selects, computes each `Replacement`, and applies the batch back
  into the target in one step. `anonymize_selections` applies a (possibly
  reviewed) set of selections.

`Deanonymizer` reverses a reversible operator (`AesEncrypt`) given the key,
recovering the original value.

```rust,ignore
use elide_operator::operators::{Erase, Mask, Replace};
use elide_redaction::{Anonymizer, Rule};

Anonymizer::new()
    .with(Rule::label(EMAIL_ADDRESS, Replace::default()))
    .with(Rule::tag("financial", Mask::stars()))
    .with(Rule::fallback(Erase))
    .anonymize(&mut document, &mut entities, &scope)
    .await?;
```

The engine is generic over the `Operator` trait (re-exported from `elide-core`)
and never names a concrete operator, so it carries no operator features — those
live on [`elide-operator`]. The `elide` facade re-exports both under
`elide::redaction` so a caller reaches the engine and the operators together.

[`elide-operator`]: ../elide-operator

## Documentation

See [`docs/`](../../docs/) for architecture, security, and API documentation.

## Changelog

See [CHANGELOG.md](../../CHANGELOG.md) for release notes and version history.
