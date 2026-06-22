# elide-stt

[![Build](https://img.shields.io/github/actions/workflow/status/nvisycom/elide/build.yml?branch=main&label=build%20%26%20test&style=flat-square)](https://github.com/nvisycom/elide/actions/workflows/build.yml)

Speech-to-text backends and transcription types for PII/PHI detection in audio.

## Overview

Audio hides the same sensitive data as text (names, numbers, addresses), but a
recognizer cannot read a waveform. Speech-to-text turns a clip into a
transcript: an ordered set of segments, each carrying the recognised words and
the time interval they occupy in the source. That transcript is what the text
recognizers detect over, and the per-segment (and per-word) timings are what map
a detected span back to the milliseconds of audio to silence or cut.

This crate provides the backend contract that turns audio bytes into transcribed
segments, with a pluggable backend so the model itself can run wherever suits
the deployment: as a hosted provider (OpenAI Whisper, Deepgram, AssemblyAI), or
a local inference engine. A no-op backend ships built in for wiring and tests;
concrete provider backends live downstream.

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
