# elide-audio

[![Build](https://img.shields.io/github/actions/workflow/status/nvisycom/elide/build.yml?branch=main&label=build%20%26%20test&style=flat-square)](https://github.com/nvisycom/elide/actions/workflows/build.yml)

Audio clip decode/encode and time-span redaction (silence, tone, remove): bytes
in, bytes out.

## Overview

An audio clip (WAV, MP3) carries sensitive speech across its timeline. This
crate opens a clip once and redacts named time spans over it: silence a span,
overlay a synthesized tone, or remove it outright, all named by a
`[start_ms, end_ms)` range. It performs no filesystem or network I/O and carries
no detection logic; a caller supplies clip bytes and receives the clip's
duration or redacted bytes.

Unlike a raster image, an audio clip is held as its encoded bytes rather than a
decoded sample buffer: it decodes to samples only on encode. WAV must re-encode
at its original sample format and bit depth, and MP3 derives its re-encode
bitrate from the original byte length, so both need the source bytes at encode
time. Redaction accumulates on the clip and applies in a single decode →
mutate → re-encode pass when the redacted bytes are produced.

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
