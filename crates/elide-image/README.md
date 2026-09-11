# elide-image

[![Build](https://img.shields.io/github/actions/workflow/status/nvisycom/elide/build.yml?branch=main&label=build%20%26%20test&style=flat-square)](https://github.com/nvisycom/elide/actions/workflows/build.yml)

Raster image decode/encode, EXIF metadata read and strip, and pixel-region
redaction: bytes in, bytes out.

## Overview

A raster image (PNG, JPEG) carries two kinds of sensitive information: the
pixels themselves (a face, a document in frame) and the metadata around them
(EXIF GPS coordinates, device serial, capture timestamp). This crate opens an
image once and does three things over it: read its embedded metadata, strip that
metadata losslessly, and paint over pixel regions (blur, pixelate, block, or
remove) named by a bounding box. It performs no filesystem or network I/O and
carries no detection logic; a caller supplies image bytes and receives metadata,
stripped bytes, or a redacted image.

The engine holds the decoded image once and re-encodes on demand, so a caller
that reads metadata, strips it, and redacts several regions pays the decode cost
a single time. It works in the toolkit's own image and redaction vocabulary, so
the codec layer drives it directly without translating a second one.

Metadata stripping is lossless where the container allows it: for JPEG and PNG
the metadata segment is removed without recompressing the pixel data, so a
stripped image is byte-faithful outside the metadata it drops, never degraded to
erase a GPS tag.

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
