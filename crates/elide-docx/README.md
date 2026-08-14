# elide-docx

[![Build](https://img.shields.io/github/actions/workflow/status/nvisycom/elide/build.yml?branch=main&label=build%20%26%20test&style=flat-square)](https://github.com/nvisycom/elide/actions/workflows/build.yml)

Standalone DOCX text extraction and byte-faithful redaction: bytes in, bytes out.

## Overview

A DOCX is a zip of OOXML parts, and its text lives across several of them: the
body, page headers and footers, footnotes and endnotes, comments, and the
glossary. This crate opens a document once and does two things over it: extract
the redactable text of every text-bearing part, each block addressed by its part
and an exact byte span, and rewrite those spans back into a new document that is
byte-for-byte identical outside the redacted text. It performs no filesystem or
network I/O and carries no detection logic; a caller supplies document bytes and
receives extracted text or a rewritten document.

Every part is typed, so a block, a replacement, and an embedding each name their
part explicitly rather than a bare path. Redaction is fail-closed: an
out-of-bounds, overlapping, or mid-character replacement, or one naming a part
that is not present or not text-bearing, refuses the whole rewrite rather than
emitting a partially-redacted document. Binary embeddings (images, objects,
fonts) are surfaced and can be replaced alongside the text in the same pass.
Extraction is partial-success: a text part that cannot be parsed is reported as
a typed issue rather than failing the whole document, so a silently un-redacted
part is made explicit.

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
