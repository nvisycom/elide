# elide-pdf

[![Build](https://img.shields.io/github/actions/workflow/status/nvisycom/elide/build.yml?branch=main&label=build%20%26%20test&style=flat-square)](https://github.com/nvisycom/elide/actions/workflows/build.yml)

Standalone PDF text extraction and born-digital redaction: bytes in, bytes out.

## Overview

A PDF holds its text as content-stream drawing operators rather than as literal
document bytes, and its images as XObject streams. This crate opens a document
once and works over it: extract each page's text and its embedded images, and
rewrite the born-digital text layer by replacing `(page, find, replace)`
occurrences and/or replacing an image's stream content, returning a new
document. It is built on the pure-Rust `lopdf` parser, so the default build has
no native dependency, and it carries no filesystem, network, or detection logic;
a caller supplies document bytes and receives extracted text and images or a
rewritten document.

Every extracted unit is typed — a text block names its page, an embedding names
its image object id. Redaction is fail-closed: a targeted text absent on its
page, or an image replacement naming an object that is not an image stream,
refuses the whole rewrite rather than emitting a partially-redacted document.
Extraction is partial-success: a page that yields no text is reported as a typed
issue (a scanned page needing OCR, or an unreadable page) rather than silently
treated as fully redacted. The optional `render` feature adds page-to-image
rendering, via the native PDFium library, for OCR of image-only pages.

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
