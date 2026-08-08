# elide-operator

[![Build](https://img.shields.io/github/actions/workflow/status/nvisycom/elide/build.yml?branch=main&label=build%20%26%20test&style=flat-square)](https://github.com/nvisycom/elide/actions/workflows/build.yml)

The shipped redaction operators for the Elide toolkit: the `Operator` library,
the key `Vault`, and the pseudonym `Generator`s.

## Overview

Once an entity is chosen for redaction, an **operator** decides *how* to hide
it: mask a phone number, replace an email, truncate a card to its last four,
encrypt a record number so it can be recovered, drop a whole table row. This
crate is the operator library — the strategies — split out from the redaction
engine (`elide-redaction`) so operators can be depended on without the
`Anonymizer`/`Deanonymizer` machinery, and vice versa.

Every operator implements the `Operator` trait from `elide-core`: it reads an
entity's value and returns a modality `Replacement`. The reversible operators
(`AesEncrypt`) also implement `ReversibleOperator`, so a `Deanonymizer` can
recover the original.

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
- **`generator`** — the pseudonym `Generator`s (e.g. `RandomToken`).

## Features

Each modality and each crypto/format operator sits behind a feature so a build
pulls only what it uses: `audio`, `image`, `tabular`, `sha2`, `hmac`, `aes`,
`datetime`, `fake`, plus `serde`/`schema` for (de)serializing operator config.

The engine that *chooses and applies* these operators is `elide-redaction`; the
whole-document orchestration over it is `elide-engine`. The `elide` facade
re-exports all three.

## Documentation

See [`docs/`](../../docs/) for architecture, security, and API documentation.

## Changelog

See [CHANGELOG.md](../../CHANGELOG.md) for release notes and version history.
