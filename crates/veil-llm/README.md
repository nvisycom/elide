# nvisy-llm

[![Build](https://img.shields.io/github/actions/workflow/status/nvisycom/runtime/build.yml?branch=main&label=build%20%26%20test&style=flat-square)](https://github.com/nvisycom/runtime/actions/workflows/build.yml)

LLM-mediated entity recognition for the Nvisy runtime, built on the
[`rig`](https://github.com/0xPlaygrounds/rig) framework.

## Overview

`LlmRecognizer` implements `EntityRecognizer<M>` against a
caller-supplied `Prompt` (the `FilePrompt` helper loads templates
from disk); pair it with any provider in `provider::*` and plug it
into a `RecognizerRegistry`. The crate doesn't impose a fixed agent
shape — the prompt + provider combination decides what the LLM is
asked to do (NER, VLM, verification).

Provider features (`openai-gpt`, `anthropic-claude`, `google-gemini`)
are independently selectable; none are on by default — the CLI/server
entry points opt in.

Speech-to-text lives in [`nvisy-stt`](../nvisy-stt) — it ships with
its own backend trait and segment-shaped output type, decoupled from
the LLM family entirely.

## Documentation

See [`docs/`](../../docs/) for architecture, security, and API documentation.

## Changelog

See [CHANGELOG.md](../../CHANGELOG.md) for release notes and version history.

## License

Apache 2.0 License, see [LICENSE.txt](../../LICENSE.txt)

## Support

- **Documentation**: [docs.nvisy.com](https://docs.nvisy.com)
- **Issues**: [GitHub Issues](https://github.com/nvisycom/runtime/issues)
- **Email**: [support@nvisy.com](mailto:support@nvisy.com)
