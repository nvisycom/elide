# Patterns and dictionaries

`PatternRecognizer` (feature `pattern`) finds entities with a known shape:
regular expressions, term lists, and checksums. It is fast, runs offline,
and is precise on structured identifiers.

```rust
use elide::recognition::pattern::PatternRecognizer;

# fn main() -> elide::Result<()> {
let patterns = PatternRecognizer::builder()
    .with_builtin_patterns()
    .with_builtin_dictionaries()
    .build_context_enhanced()?;
# Ok(())
# }
```

The built-in patterns cover emails, phone numbers, payment cards, IBANs and
bank accounts, IP and MAC addresses, URLs, dates, and national identifiers
for many countries. Validators reject look-alikes: a card number must pass
the Luhn check, an IBAN its checksum. Country-specific patterns run only when
the [scope](analyzers.md#the-scope) names that country, and language-specific
ones only for that language.

## Context

`build_context_enhanced` wraps the recognizer so that keywords near a match
raise its confidence: a nine-digit number is more likely an identity number
next to "SSN" than on its own. Use `build` for the recognizer without it.

## Your own patterns

A rule names its labels, from most to least specific, and one or more
variants, each a regex with a score:

```rust
use elide::recognition::pattern::{PatternRecognizer, Regex};

# fn main() -> elide::Result<()> {
let employee_id = Regex::from_toml(r#"
    name = "employee-id"
    labels = ["internal_id"]

    [[variants]]
    regex = '\bEMP-\d{5}\b'
    score = 0.95
"#)?;

let patterns = PatternRecognizer::builder()
    .with_pattern(employee_id)
    .with_builtin_patterns()
    .build()?;
# Ok(())
# }
```

Rules are TOML so they can live in configuration; `Regex::builder()` builds
one in code. A rule can also carry context keywords, a validator, and the
languages and countries it applies to.

## Dictionaries

A dictionary matches a list of terms, such as product names or internal
code names, under one label:

```rust
use elide::recognition::pattern::{Dictionary, PatternRecognizer, Term};

# fn main() -> elide::Result<()> {
let products = Dictionary::metadata_from_toml(r#"
    name = "products"
    labels = ["internal_id"]
    score = 0.8
"#)?
.with_terms(Term::from_text("Project Falcon\nProject Osprey"))
.build()?;

let patterns = PatternRecognizer::builder()
    .with_dictionary(products)
    .build()?;
# Ok(())
# }
```

`Term::from_csv` reads terms from CSV, where each column can carry its own
score.
