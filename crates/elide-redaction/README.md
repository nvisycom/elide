# elide-redaction

[![Build](https://img.shields.io/github/actions/workflow/status/nvisycom/elide/build.yml?branch=main&label=build%20%26%20test&style=flat-square)](https://github.com/nvisycom/elide/actions/workflows/build.yml)

The redaction engine for PII/PHI: the `Anonymizer`, `Deanonymizer`, and the
shipped operators.

## Overview

Once entities are detected, they have to be hidden — and the *how* is a policy
decision: mask a phone number, replace an email, encrypt a record number so it
can be recovered, drop a whole table row. This crate is the "hide" engine.

`Anonymizer` is the redaction counterpart to the detection `Analyzer`. It holds
an ordered list of selection rules (bind an operator to a label, a tag, a
predicate, or a catch-all fallback) and two entry points: `anonymize` picks each
entity's operator, computes its replacement, and applies the batch back into the
target in one step; `plan` stops a step short and hands back the `Redactions`
batch for inspection or deferred application.

```rust,ignore
use elide_redaction::Anonymizer;
use elide_redaction::redaction::operators::{Mask, Replace, Erase};

Anonymizer::new()
    .with_label(EMAIL_ADDRESS, Replace::default())
    .with_tag("financial", Mask::stars())
    .with_fallback(Erase)
    .anonymize(&mut document, &mut entities)
    .await?;
```

The shipped operators model Presidio's set, generalised to be multimodal:
`Mask`, `Replace`, `Hash`, `Pseudonymize`, `Erase`, and `Keep` work everywhere;
`DropRow`/`DropColumn` (tabular), `Blur`/`Pixelate`/`Blackbox` (image), and
`Silence`/`Beep` (audio) are feature-gated by modality. The reversible `Encrypt`
operator (feature `crypto`, AES-256-GCM) replaces a value with a ciphertext the
`Deanonymizer` can recover given the key; the locale-aware `Fake` operator
(feature `fake`) swaps in plausible synthetic values.

Pseudonymization draws a stable surrogate per entity from a `generator`, kept
consistent across coreferent mentions through a `Vault`; `crypto` keys come from
a `key_provider`.

## Documentation

See [`docs/`](../../docs/) for architecture, security, and API documentation.

## Changelog

See [CHANGELOG.md](../../CHANGELOG.md) for release notes and version history.
