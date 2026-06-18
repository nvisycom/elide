# elide

[![Build](https://img.shields.io/github/actions/workflow/status/nvisycom/elide/build.yml?branch=main&label=build%20%26%20test&style=flat-square)](https://github.com/nvisycom/elide/actions/workflows/build.yml)

Composable toolkit for detecting and redacting sensitive data.

elide is a Rust toolkit for finding and removing PII and PHI from
documents. It provides the building blocks (recognizers, deduplication,
validation, redaction, and format handling) that a consumer wires into
their own document-processing flow. elide is the toolkit layer only; the
orchestrating runtime and gateway server live in separate projects.

> [!WARNING]
> **Active development: API not stable.** This project is under active
> development. Public APIs, configuration shapes, and on-disk formats may
> change without notice between releases. Pin a specific commit if you
> depend on this in production.

## Features

- **Pattern detection**: regex, dictionary, and checksum recognizers find structured PII and PHI across many common formats and jurisdictions
- **Context-aware scoring**: nearby keywords lift the confidence of ambiguous matches, so weak findings clear the threshold only when their surroundings support them
- **Deduplication**: overlapping findings from multiple recognizers reconcile into a single set of entities, with conflict resolution and confidence calibration
- **Redaction operators**: mask, replace, hash, or encrypt each detected entity, with the reversible options recording what is needed to restore it
- **Format codecs**: read, edit, and write documents (plain text, JSON, HTML, XML, and more) with faithful round-tripping that changes only the redacted parts
- **Provenance-first model**: every entity carries its full audit trail of how it was found, scored, and hidden

## Documentation

See [`docs/`](docs/) for architecture, security, and API documentation.

## Changelog

See [CHANGELOG.md](CHANGELOG.md) for release notes and version history.

## License

Apache 2.0 License, see [LICENSE.txt](LICENSE.txt)

## Support

- **Documentation**: [docs.nvisy.com](https://docs.nvisy.com)
- **Issues**: [GitHub Issues](https://github.com/nvisycom/elide/issues)
- **Email**: [support@nvisy.com](mailto:support@nvisy.com)
