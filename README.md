# Veil

[![Build](https://img.shields.io/github/actions/workflow/status/nvisycom/veil/build.yml?branch=main&label=build%20%26%20test&style=flat-square)](https://github.com/nvisycom/veil/actions/workflows/build.yml)

Composable toolkit for detecting and redacting sensitive data.

A Rust toolkit for PII/PHI detection and redaction — recognizers,
deduplication layers, validation checks, and redaction strategies — that
a consumer plugs into their own document-processing flow. Veil is the
toolkit layer only; the orchestrating runtime and gateway server live in
separate projects.

> [!WARNING]
> **Active development: API not stable.** This project is under active
> development. Public APIs, configuration shapes, and on-disk formats may
> change without notice between releases. Pin a specific commit if you
> depend on this in production.

## Crates

- **veil-core:** Domain types, traits, and errors
- **veil-toolkit:** Composable recognizer/redaction registries, dedup layers, and validation checks

## Documentation

See [`docs/`](docs/) for architecture, security, and API documentation.

## Changelog

See [CHANGELOG.md](CHANGELOG.md) for release notes and version history.

## License

Apache 2.0 License, see [LICENSE.txt](LICENSE.txt)

## Support

- **Documentation**: [docs.nvisy.com](https://docs.nvisy.com)
- **Issues**: [GitHub Issues](https://github.com/nvisycom/veil/issues)
- **Email**: [support@nvisy.com](mailto:support@nvisy.com)
