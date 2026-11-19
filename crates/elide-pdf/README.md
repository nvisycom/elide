# elide-pdf

[![Build](https://img.shields.io/github/actions/workflow/status/nvisycom/elide/build.yml?branch=main&label=build%20%26%20test&style=flat-square)](https://github.com/nvisycom/elide/actions/workflows/build.yml)

The PDF codec for elide: decode a PDF for detection, and encode it back with
every detection redacted.

## Overview

A PDF holds its text as content-stream drawing operators, and its images and
attached files as streams of their own. Decoding first rebuilds the document,
copying only what draws its pages, its internal links, and its attached files,
each object with only the keys the PDF specification defines for it; whatever
it leaves behind (metadata, annotations, scripts, hidden layers, prior
revisions, private application data) is recorded, so detection reads only what
will ship. It then yields each page's text, decoded glyph by glyph, (with
`render`) each scanned page's render as an image for OCR, and surfaces every
embedded image and attached file as content of its own format for the registry
to redact.

The rebuilt document is untagged: its structure tree carries alternate text
detection never reads. Embedded font programs are copied as stored.

Encoding deletes the detected glyphs from the content streams, keeping a
selectable text layer, writes the redacted images and files back, and replaces
a scanned page by its render as the document then draws it, with each region
redacted on it painted over. A scanned page is an image part like any other:
an orchestrator analyzes it only when an image pipeline is registered, and
passes it through untouched otherwise.
With the `render` feature and `RasterMode::Always`, every page is instead
rendered via PDFium and replaced by an image with the detected text painted
over. Redaction fails closed: a page that cannot be read, text that cannot be
located, or a part the document never surfaced refuses the whole document
rather than shipping anything unredacted.

The default build is pure Rust, reading through `hayro-syntax` and writing
through `pdf-writer`, with no native dependency.

## Documentation

See [`docs/`](../../docs/) for architecture, security, and API documentation.

## Changelog

See [CHANGELOG.md](../../CHANGELOG.md) for release notes and version history.

## License

Apache 2.0 License, see [LICENSE](../../LICENSE). It bundles Adobe's CMap
resources through `hayro-cmap`, whose notice is in
[THIRD-PARTY-NOTICES.md](../../THIRD-PARTY-NOTICES.md).

## Support

- **Documentation**: [docs.nvisy.com](https://docs.nvisy.com)
- **Issues**: [GitHub Issues](https://github.com/nvisycom/elide/issues)
- **Email**: [support@nvisy.com](mailto:support@nvisy.com)
