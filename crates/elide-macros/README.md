# elide-macros

[![Build](https://img.shields.io/github/actions/workflow/status/nvisycom/elide/build.yml?branch=main&label=build%20%26%20test&style=flat-square)](https://github.com/nvisycom/elide/actions/workflows/build.yml)

Derive macros for the Elide toolkit: component identity and the backend
request/response contract.

## Overview

Use the derives through `elide-core`, which re-exports each next to the trait
it implements, or through the `elide` facade; this crate is not a direct
dependency.

- **`Component`** — the identity a recognizer, enricher, or operator records
  in the audit trail. By default the deriving crate's name and version;
  `#[component(name = "...", version = "...")]` fixes either, a field marked
  `#[component(name)]` supplies the name, and a field marked
  `#[component(delegate)]` supplies the whole identity of a wrapped component.
- **`Request`** — a backend request, its language read from the field
  marked `#[backend(language)]`.
- **`Response`** — a backend response, the model that produced it read
  from the field marked `#[backend(model)]`.

```rust,ignore
use elide_core::Component;

/// A fixed name and algorithm version.
#[derive(Component)]
#[component(name = "mask", version = "1.0.0")]
pub struct Mask;

/// The deriving crate's name and version.
#[derive(Component)]
pub struct MyRecognizer;

/// A configurable name, the crate's version.
#[derive(Component)]
pub struct NamedRecognizer {
    #[component(name)]
    name: String,
}

/// The wrapped component's identity.
#[derive(Component)]
pub struct Logged<R> {
    #[component(delegate)]
    inner: R,
}
```

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
