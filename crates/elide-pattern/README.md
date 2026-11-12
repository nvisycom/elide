# elide-pattern

[![Build](https://img.shields.io/github/actions/workflow/status/nvisycom/elide/build.yml?branch=main&label=build%20%26%20test&style=flat-square)](https://github.com/nvisycom/elide/actions/workflows/build.yml)

Regex and dictionary recognizers for PII/PHI detection.

## Overview

Many kinds of sensitive data have a recognizable shape (a credit-card number, an
email address, a national ID) or appear as known terms (a list of currencies,
nationalities, or brand names). This crate detects both: regular-expression
rules for structured formats, and dictionaries for fixed sets of literal terms.
A single pass over the text runs every rule and reports what it found, with a
confidence score for each match.

Matches that have a definite structure can be checked against a validator before
being reported, so values that merely look right but fail their checksum (an
invalid IBAN, a malformed SSN) are dropped. A broad set of patterns,
dictionaries, and validators ship built in, covering common formats across
several jurisdictions, and you can add your own alongside them.

Some of the shipped patterns and validators are adapted from
[Microsoft Presidio](https://github.com/microsoft/presidio).

```rust
use elide_core::modality::text::{Text, TextData};
use elide_core::recognition::{Context, Recognizer, Scope, Subject};
use elide_pattern::{Pattern, PatternRecognizer};

# #[tokio::main(flavor = "current_thread")]
# async fn main() -> elide_core::Result<()> {
let employee_id = Pattern::from_toml(r#"
    name = "employee_id"
    labels = ["internal_id"]

    [[variants]]
    regex = '\bEMP-\d{5}\b'
    score = 0.95
"#)?;

let recognizer = PatternRecognizer::builder()
    .with_builtin_patterns()
    .with_builtin_dictionaries()
    .with_pattern(employee_id)
    .build()?;

let scope = Scope::new();
let subject = Subject::<Text>::new(TextData::new("EMP-12345, jane@example.com"));
let entities = recognizer.recognize(&subject, &Context::new(&scope)).await?;
let labels: Vec<&str> = entities.iter().map(|e| e.label.as_str()).collect();
assert!(labels.contains(&"internal_id"));
assert!(labels.contains(&"email_address"));
# Ok(())
# }
```

The crate holds:

- **`PatternRecognizer`** — the recognizer, built with
  `PatternRecognizer::builder()`. It scans the text once with every regex and
  once with every dictionary term, then lifts each match whose rule's context
  is found nearby.
- **`Pattern`** and **`Dictionary`** — the two rule types, written in TOML or
  with a builder. A pattern holds regex `Variant`s, each with a score and an
  optional validator; a dictionary holds literal `Term`s, loaded from a list or
  a CSV, scored uniformly or per column.
- **`context`** — a rule's `[context]`: keywords, per language or in any, and
  an anchor label, that raise a nearby match's confidence. Keywords match as
  case-insensitive substrings (`SubstringMatcher`), or by lemma when the text
  was tokenized (`LemmaMatcher`).
- **`validators`** — checksum and structure checks (Luhn, IBAN, national IDs
  across many jurisdictions), resolved by name from a `ValidatorRegistry`.
- **`patterns`** and **`dictionaries`** — the shipped rules, by region and by
  language.

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
