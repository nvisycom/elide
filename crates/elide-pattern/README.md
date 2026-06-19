# elide-pattern

[![Build](https://img.shields.io/github/actions/workflow/status/nvisycom/elide/build.yml?branch=main&label=build%20%26%20test&style=flat-square)](https://github.com/nvisycom/elide/actions/workflows/build.yml)

Regex and dictionary recognizers for PII/PHI detection.

## Overview

Many kinds of sensitive data have a recognizable shape (a credit-card
number, an email address, a national ID) or appear as known terms (a
list of currencies, nationalities, or brand names). This crate detects
both: regular-expression rules for structured formats, and dictionaries
for fixed sets of literal terms. A single pass over the text runs every
rule and reports what it found, with a confidence score for each match.

Matches that have a definite structure can be checked against a
validator before being reported, so values that merely look right but
fail their checksum (an invalid IBAN, a malformed SSN) are dropped. A
broad set of patterns, dictionaries, and validators ship built in,
covering common formats across several jurisdictions, and you can add
your own alongside them.

Some of the shipped patterns and validators are adapted from
[Microsoft Presidio](https://github.com/microsoft/presidio).

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
