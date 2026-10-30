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
    .with_builtin_walks()
    .build()?;
# Ok(())
# }
```

The built-in patterns cover emails, phone numbers, payment cards, IBANs and
bank accounts, IP and MAC addresses, URLs, dates, and national identifiers
for many countries. Validators reject look-alikes: a card number must pass
the Luhn check, an IBAN its checksum. Country-specific patterns run only when
the [scope](analyzers.md#the-scope) names that country, and language-specific
ones only for that language.

## Names

The built-in walks find names next to a word that marks them:

- **Companies** by their legal form: "Müller Bau GmbH", "Bank of America,
  N.A.", "Kowalski Budownictwo sp. z o.o.". The recognizer reads back from
  the legal form while the words read as a name, and stops at a sentence
  end, a line break, an amount, or an earlier company ("Acme GmbH and Beta
  GmbH" is two names). Contract wording in front of a name ("by and
  between", "the Seller") is left out.
- **Institutions** around a head word: "U.S. Department of Justice",
  "Supreme Court of Canada". Generic phrases such as "Audit Committee" or
  "Board of Directors" are not institutions.
- **People** after a title: "Dr. Hans Müller", "Mme Claire Dubois". The
  title stays visible; only the name is reported.

Markers that are also ordinary words, such as `AG`, `Office`, or `Judge`,
need a word like "company" nearby.

Languages that write names partly in lowercase, such as Czech's "Základní
škola Brno, s.r.o.", need the document's language: named in the
[scope](analyzers.md#the-scope), or found by a language-detection enricher.
Without one, a lowercase word after the first capitalized one ends the name.

## Context

A rule can name context keywords, and a keyword near a match raises its
confidence: a nine-digit number is more likely an identity number next to
"SSN" than on its own. Keywords match whole words by default; `match =
"substring"` in a rule's `[context]` also matches them inside longer words.
`with_keyword_matcher(LemmaMatcher)` on the builder also matches their other
word forms when the text was tokenized, under the same word-boundary rule.

## Your own patterns

A rule names its labels, from most to least specific, and one or more
variants, each a regex with a score:

```rust
use elide::recognition::pattern::{PatternRecognizer, Pattern};

# fn main() -> elide::Result<()> {
let employee_id = Pattern::from_toml(r#"
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

Rules are TOML so they can live in configuration; `Pattern::builder()` builds
one in code. A rule can also carry context keywords, a validator, and the
languages and countries it applies to.

The `regex` crate has no lookahead or lookbehind. To require text around a
match without including it, name the part to report `entity`:

```toml
regex = 'badge:\s*(?P<entity>\d{5})\b'
```

This reports `12345` in `badge: 12345`. A validator sees only the `entity`
part.

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
