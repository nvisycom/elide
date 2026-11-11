# elide-redaction

[![Build](https://img.shields.io/github/actions/workflow/status/nvisycom/elide/build.yml?branch=main&label=build%20%26%20test&style=flat-square)](https://github.com/nvisycom/elide/actions/workflows/build.yml)

Redaction for PII/PHI: the shipped operators, and the `Anonymizer` and
`Deanonymizer` that select and apply them.

## Overview

Once entities are detected, they have to be hidden — and the *how* is a policy
decision: mask a phone number, replace an email, encrypt a record number so it
can be recovered, drop a whole table row. An **operator** is one such strategy:
it reads an entity's value and returns a modality `Replacement`. Every operator
implements the `Operator` trait from `elide-core`; the reversible ones
(`AesEncrypt`) also implement `ReversibleOperator`.

`Anonymizer` is the redaction counterpart to the detection `Analyzer`. It holds
an ordered list of selection `Rule`s (bind an operator to a label, a tag, a
predicate over a `MatchContext`, or a catch-all fallback) and two paths:

- `pick` runs the rules and records each entity's decision as a `Selection`
  audit event (the operator, how much its output leaks, the rule that matched,
  and the policy's rationale) without reading any data: the decision phase,
  inspectable before anything is applied, while the entities can still be
  edited.
- `anonymize` resolves the same decisions, records any not already on an
  entity's trail, computes each `Replacement`, applies the batch back into the
  target, and records a `Redaction` naming the operator that ran.

`Deanonymizer` reverses a reversible operator given the key, recovering the
original value.

```rust,ignore
use elide_redaction::operators::{Erase, Mask, Replace};
use elide_redaction::{Anonymizer, Rule};

Anonymizer::new()
    .with(Rule::label(EMAIL_ADDRESS, Replace::default()))
    .with(Rule::tag("financial", Mask::stars()))
    .with(Rule::fallback(Erase))
    .anonymize(&mut document, &mut entities, &scope)
    .await?;
```

## What's here

- **`operators`** — the shipped operators, grouped by modality:
  - Text/tabular: `Mask`, `Replace`, `Truncate`, `Clamp`, `Pseudonymize`,
    `GeneralizeDate` (feature `datetime`), `Sha2Hash` (`sha2`), `HmacHash`
    (`hmac`), `AesEncrypt` (`aes`), `Fake` (`fake`).
  - Tabular: `DropRow`, `DropColumn` (feature `tabular`).
  - Image: `Blur`, `Pixelate`, `Blackbox` (feature `image`).
  - Audio: `Silence`, `Beep` (feature `audio`).
  - Cross-modality: `Erase`, `Keep`, and `WithFallback` for composing a
    declining `TryOperator` with a fallback.
- **`vault`** — the `Vault` trait and the default `InMemoryVault`, backing the
  stable per-entity surrogates `Pseudonymize` produces.
- **`generator`** — the pseudonym generators (e.g. `RandomToken`).

## Features

Each modality and each crypto/format operator sits behind a feature so a build
pulls only what it uses: `audio`, `image`, `tabular`, `metadata`, `sha2`,
`hmac`, `aes`, `datetime`, `fake`, plus `serde`/`schema` for (de)serializing
operator config.

The whole-document orchestration over this crate is `elide-engine`. The `elide`
facade re-exports both, under `elide::redaction` for this one.

## Documentation

See [`docs/`](../../docs/) for architecture, security, and API documentation.

## Changelog

See [CHANGELOG.md](../../CHANGELOG.md) for release notes and version history.
