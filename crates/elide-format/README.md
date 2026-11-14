# elide-format

[![Build](https://img.shields.io/github/actions/workflow/status/nvisycom/elide/build.yml?branch=main&label=build%20%26%20test&style=flat-square)](https://github.com/nvisycom/elide/actions/workflows/build.yml)

The format registry that assembles the Elide codecs: resolve a file to a handler
and decode it.

## Overview

`elide-codec` defines the codec contracts: the `Loader` and `DocumentLoader`
traits, the `Stream` a decoded part is read and redacted through, and the
`Format` descriptor that names a loader. The concrete formats live with their
engines: TXT, JSON and CSV in `elide-plain`, HTML and XML in `elide-markup`,
and the rich-media formats in `elide-image`, `elide-audio`, `elide-office` and
`elide-pdf`. This crate assembles them.

`FormatRegistry` indexes formats by id, file extension and content type, and
decodes raw bytes through the matching loader. `FormatRegistry::with_builtin`
registers exactly the formats the enabled features compile in, so a caller
resolves a document from a filename or MIME type without naming the crate that
handles it. With the `sniff` feature, content that carries neither resolves
from its magic bytes. Custom formats register at runtime, and cloning a
registry is cheap, so build it once and share it.

```rust
use elide_codec::content::ContentData;
use elide_core::modality::text::Text;
use elide_core::redaction::HiddenContentPolicy;
use elide_format::FormatRegistry;

# #[cfg(not(feature = "txt"))]
# fn main() {}
# #[cfg(feature = "txt")]
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> elide_core::Result<()> {
let registry = FormatRegistry::with_builtin();
let content = ContentData::from_text("Contact alice@example.com").with_filename("notes.txt");
let mut document = registry
    .decode_content(content, HiddenContentPolicy::default())
    .await?;

assert_eq!(document.format_id().as_str(), "elide.text.txt");
assert!(document.stream_mut::<Text>("body").is_some());
# Ok(())
# }
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
