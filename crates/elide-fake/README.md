# elide-fake

[![Build](https://img.shields.io/github/actions/workflow/status/nvisycom/elide/build.yml?branch=main&label=build%20%26%20test&style=flat-square)](https://github.com/nvisycom/elide/actions/workflows/build.yml)

Locale-aware fake-data redaction operator for PII/PHI.

## Overview

Masking or erasing a value protects it, but destroys the document's shape: a
form with every name blacked out no longer reads, demos, or tests like a real
one. This crate provides `Fake`, a redaction operator that instead swaps each
detected entity for a *plausible* fake — a real-looking name, address, IBAN, or
date — drawn from the [`fake`](https://docs.rs/fake) crate's locale tables, so
the redacted output stays usable while the original value is gone.

The locale is chosen per entity from its BCP-47 `language` (the field a
language-aware recognizer stamps), falling back to a configurable default. RNG
state is derived from the entity's coreference id (or its UUID when it has none),
so coreferent mentions of the same real-world thing collapse to the same fake
within a run, and a fixed seed makes a run reproducible.

Structured labels (IBAN, payment card, postal code, phone, date) are
*pattern-preserving*: the fake matches the original's length and character-class
layout, only the digits and letters change. Free-form labels (names, addresses,
organizations) emit a fresh locale-aware fake whose length need not match. A
label outside the supported set delegates to a fallback operator supplied at
construction, so `Fake` slots into a policy alongside `Mask`, `Replace`, or
`Erase` for everything it doesn't fake itself. It applies to both text and
tabular cells.

```rust,ignore
use elide::redaction::operators::Mask;
use elide_fake::Fake;

// Fake known PII; mask anything else.
let op = Fake::new(Mask::stars()).with_seed(42);
```

## Documentation

See [`docs/`](../../docs/) for architecture, security, and API documentation.

## Changelog

See [CHANGELOG.md](../../CHANGELOG.md) for release notes and version history.

## License

Apache 2.0 License, see [LICENSE.txt](../../LICENSE.txt)

## Support

- **Documentation**: [docs.nvisy.com](https://docs.nvisy.com)
- **Issues**: [GitHub Issues](https://github.com/nvisycom/elide/issues)
- **Email**: [support@nvisy.com](mailto:support@nvisy.com)
