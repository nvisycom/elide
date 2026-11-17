# elide-pdf

[![Build](https://img.shields.io/github/actions/workflow/status/nvisycom/elide/build.yml?branch=main&label=build%20%26%20test&style=flat-square)](https://github.com/nvisycom/elide/actions/workflows/build.yml)

The PDF codec for elide: decode a PDF for detection, and encode it back with
every detection redacted.

## Overview

A PDF holds its text as content-stream drawing operators, and its images and
attached files as streams of their own. Decoding first strips the document's
hidden content (metadata, annotations, scripts, hidden layers, prior
revisions), so detection reads only what will ship. It then yields each page's
text, decoded glyph by glyph, and surfaces every embedded image, attached file,
and (with `render`) scanned page as content of its own format for the registry
to redact.

Encoding deletes the detected glyphs from the content streams, keeping a
selectable text layer, and writes the redacted images, files, and pages back.
With the `render` feature and `RasterMode::Always`, every page is instead
rendered via PDFium and replaced by an image with the detected text painted
over. Redaction fails closed: a page that cannot be read, text that cannot be
located, or a part the document never surfaced refuses the whole document
rather than shipping anything unredacted.

The default build is pure Rust on `lopdf`, with no native dependency.

## Documentation

See [`docs/`](../../docs/) for architecture, security, and API documentation.

## Changelog

See [CHANGELOG.md](../../CHANGELOG.md) for release notes and version history.

## License

Apache 2.0 License, see [LICENSE](../../LICENSE)

## Support

- **Documentation**: [docs.nvisy.com](https://docs.nvisy.com)
- **Issues**: [GitHub Issues](https://github.com/nvisycom/elide/issues)
- **Email**: [support@nvisy.com](mailto:support@nvisy.com)
