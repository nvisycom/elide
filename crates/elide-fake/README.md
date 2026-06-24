# nvisy-fake

[![Build](https://img.shields.io/github/actions/workflow/status/nvisycom/runtime/build.yml?branch=main&label=build%20%26%20test&style=flat-square)](https://github.com/nvisycom/runtime/actions/workflows/build.yml)

Locale-aware fake-data anonymizer for the Nvisy runtime.

## Overview

`Fake` is a text-modality `Anonymizer` that swaps a detected entity
for a plausible fake value drawn from the [`fake`](https://docs.rs/fake)
crate's locale tables. The locale is picked per-entity from the
entity's BCP-47 `language` field, falling back to the
`default_language` passed at construction when no tag is present.

Construct with `Fake::new(language_tag)`; tune behaviour with
`.with_seed(u64)`, `.length_preserving()`, and `.format_preserving()`.
RNG state is derived from `entity_id` (or the UUID when no
coreference id is present) so coreferent mentions collapse to the
same fake value within a run.

Entity kinds outside the core PII set (`PersonName`, `EmailAddress`,
`PhoneNumber`, `Address`, `PostalCode`, `Url`, `DateOfBirth`, `Age`,
`PaymentCard`, `Iban`, `BankAccount`, `Currency`) fall through to a
`[{entity_kind}]` placeholder.

## Documentation

See [`docs/`](../../docs/) for architecture, security, and API documentation.

## Changelog

See [CHANGELOG.md](../../CHANGELOG.md) for release notes and version history.

## License

Apache 2.0 License, see [LICENSE.txt](../../LICENSE.txt)

## Support

- **Documentation**: [docs.nvisy.com](https://docs.nvisy.com)
- **Issues**: [GitHub Issues](https://github.com/nvisycom/runtime/issues)
- **Email**: [support@nvisy.com](mailto:support@nvisy.com)
