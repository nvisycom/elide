# elide-context

[![Build](https://img.shields.io/github/actions/workflow/status/nvisycom/elide/build.yml?branch=main&label=build%20%26%20test&style=flat-square)](https://github.com/nvisycom/elide/actions/workflows/build.yml)

Keyword-based confidence boosting for detected entities.

## Overview

Some matches are ambiguous on their own but become far more likely when the
surrounding text gives them away. A nine-digit number is just a number until the
word "SSN" sits beside it. This crate raises the confidence of a detected entity
when configured keywords appear nearby, so weak-but-plausible findings clear the
threshold only when their context supports them.

A recognizer declares the keywords it cares about and how close they need to be.
After recognition runs, the enhancer scans the neighbourhood of each entity for
those keywords and lifts its confidence on a hit. It can match keywords as plain
substrings, or, when token information is available, match across word variants
such as "running" and "run".

The boost itself is recorded on the entity, so its effect on the final
confidence is always traceable.

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
