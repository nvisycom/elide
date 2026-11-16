# elide-plain

[![Build](https://img.shields.io/github/actions/workflow/status/nvisycom/elide/build.yml?branch=main&label=build%20%26%20test&style=flat-square)](https://github.com/nvisycom/elide/actions/workflows/build.yml)

The plain-text formats: TXT, JSON and CSV.

## Overview

Most formats need an engine of their own: an image decoder, an audio codec, a
PDF parser. TXT, JSON and CSV are text all the way down, so their handlers need
only the codec contracts in `elide-codec`. They live here; HTML and XML live in
`elide-markup`.

Each format is a `Format` built by `txt_format`, `json_format` or `csv_format`
(`csv_format_with` sets the header row and delimiter), each behind its own
feature. `elide-format`'s `FormatRegistry::with_builtin` registers the enabled
ones. A decoded document is one stream that recognizers read and redactions are
written back into:

- **TXT** is one `Text` stream chunked by paragraph, so a multi-line match (a
  PEM block) is one chunk. Redaction splices the text in place, and every other
  byte is kept.
- **JSON** is one `Text` stream with a chunk per key, string and scalar, each
  with its enclosing key as a context hint. A redaction is spliced into the
  source at the escaped position, so formatting and escapes outside it are
  kept. A scalar redacted into something that is not a JSON literal becomes a
  string.
- **CSV** is one `Tabular` stream with a chunk per cell, each hinted by its
  column header. Cells are redacted in place, or a row or column dropped, and
  the table is written back with its delimiter and line endings. Quoting is
  normalized: a field is quoted only where it needs to be.

A location a document cannot take (out of range, splitting a character, a cell
it doesn't have) fails the write rather than being skipped.

```rust
# #[cfg(not(feature = "txt"))]
# fn main() {}
# #[cfg(feature = "txt")]
# #[tokio::main(flavor = "current_thread")]
# async fn main() -> elide_core::Result<()> {
use elide_codec::content::ContentData;
use elide_core::modality::DataWriter;
use elide_core::modality::text::{Text, TextLocation, TextReplacement};
use elide_core::redaction::{HiddenContentPolicy, Redactions};

let mut document = elide_plain::txt_format()
    .decode(
        ContentData::from_text("Contact alice@example.com"),
        HiddenContentPolicy::default(),
    )
    .await?;

let mut body = document.stream_mut::<Text>("body").expect("one text stream");
body.write_at(Redactions::from([(
    TextLocation::inline(8..25),
    TextReplacement::substituted("[EMAIL]"),
)]))
.await?;

assert_eq!(document.encode()?.as_bytes(), b"Contact [EMAIL]");
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
