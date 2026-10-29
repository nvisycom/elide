# Installation

Elide needs Rust 1.95 or later. Add it with the features you need, and tokio
to run it:

```toml
[dependencies]
elide = { git = "https://github.com/nvisycom/elide", features = ["codec-txt"] }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

The default features are the three recognizer families: `pattern`, `ner`,
and `llm`. No format codec is enabled by default; add one for each format you
read, such as `codec-txt` or `codec-docx`, or a group such as `codec-text`.
A codec feature also enables its modality (`codec-png` enables `image`), but
not the other way round: the `image`, `audio`, and `tabular` features alone
add no formats.

Every entry point is `async`, so you need a runtime. Elide does not start one
of its own.

The [feature reference](features.md) lists every flag. Continue with
[Your first redaction](first-redaction.md) to detect and redact entities in a
file.
