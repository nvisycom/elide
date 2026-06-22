# elide-ocr

[![Build](https://img.shields.io/github/actions/workflow/status/nvisycom/elide/build.yml?branch=main&label=build%20%26%20test&style=flat-square)](https://github.com/nvisycom/elide/actions/workflows/build.yml)

OCR backends and recognized-text types for PII/PHI detection in images.

## Overview

Images hide the same sensitive data as text — names, numbers, addresses on
a scanned form or a screenshot — but a recognizer cannot read pixels. OCR
turns an image into recognized text: an ordered set of blocks, each
carrying the text and the bounding boxes the words occupy in the image.
That text is what the text recognizers detect over, and the per-word boxes
are what map a detected span back to the image region to blur, block, or
remove.

This crate provides the backend contract that turns image bytes into
recognized text blocks, with a pluggable backend so the engine itself can
run wherever suits the deployment: as a hosted document-AI service (Google
Document AI, Azure, AWS Textract), or a local engine. A no-op backend ships
built in for wiring and tests; concrete engine backends live downstream.

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
