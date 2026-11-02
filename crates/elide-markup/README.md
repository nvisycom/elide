# elide-markup

[![Build](https://img.shields.io/github/actions/workflow/status/nvisycom/elide/build.yml?branch=main&label=build%20%26%20test&style=flat-square)](https://github.com/nvisycom/elide/actions/workflows/build.yml)

The markup format handlers, HTML and XML, and the character reference decoder
they read text with.

## Overview

HTML and XML differ in their parser and serializer but share the same
redactable units (text runs, attribute values, comments) and the same
bookkeeping. One engine reads both over `elide-codec`'s extract engine
(`ExtractedItem`, `ExtractStream`, `SourceAddresser`): it splits a document into
passages a reader takes in as one, decodes their character references so
detection reads what a reader sees, and writes back only the stretch of each
value a redaction changed, escaped for where it lands. Everything outside a
redaction stays byte-identical.

Before a document is read, a sanitizer removes its hidden content under the
caller's `HiddenContentPolicy`: scripts and other active content, event
handlers, remote loads, comments, and metadata. HTML is read as a browser
reads it — tag names and attributes by the HTML tokenizer's rules, URLs and
text by HTML's character reference rules — and a document a browser would read
differently from this engine is refused.

`html_format()` and `xml_format()` are behind the `html` and `xml` features;
`elide-format`'s `FormatRegistry::with_builtin` wires the enabled ones in. With
no feature, the crate is only `decode_references`, which a format that reads
XML another way (`elide-office`'s OOXML parts) decodes its text with.

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
