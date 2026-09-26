# elide-wasm

[![Build](https://img.shields.io/github/actions/workflow/status/nvisycom/elide/build.yml?branch=main&label=build%20%26%20test&style=flat-square)](https://github.com/nvisycom/elide/actions/workflows/build.yml)

WebAssembly bindings that run the Elide detect-and-redact pipeline in a browser.

## Overview

A thin WebAssembly boundary over the Elide facade. It exposes the pipeline as a
set of opaque handles a caller composes from JavaScript — recognizers (built
from a config or a JS callback) and enrichers folded per modality onto a
`PipelineBuilder` — and one `redact` call that runs the resulting pipeline over
a blob, dispatching by the format hint to the matching detect-and-redact stage
and returning the redacted bytes plus the entities found. It carries no
detection logic of its own; the whole pipeline is the same one the native
toolkit runs.

Every modality the browser can hand over as bytes is covered: text and tabular
(pattern and NER recognizers scan the decoded text or cells), image (its EXIF
metadata is scrubbed, and with an OCR enricher its rendered text is read and
matched regions blacked out), and audio (with an STT enricher its transcript is
scanned and matched spans silenced). The models the browser supplies — NER, OCR,
STT — arrive as async JavaScript callbacks; language detection is a built-in
pure-Rust enricher.

The pipeline is asynchronous, and in the browser its work is driven by the
page's own event loop rather than a Rust async runtime, so nothing here spawns
threads, opens sockets, or touches a filesystem. Only the parts of the toolkit
that compile to WebAssembly are pulled in — detection and redaction, the
plain-text, image, and WAV codecs, and the pure-Rust enrichers; the native
codecs and engines (MP3, PDF, office formats, in-process OCR/STT/LLM models) are
deliberately left out of the browser build, their models delegated to JavaScript
instead.

The rich Rust objects stay in wasm memory behind the handles, and only the
config and the redaction result cross the boundary as data, typed through
generated TypeScript definitions so the browser code stays fully typed. Build
the package with `make wasm-pkg`; the in-browser demo that consumes it lives
alongside this crate and is published to GitHub Pages when a push to `main`
touches the wasm sources (or on demand via the workflow).

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
