# Shipped asset tree

The `elide-pattern` crate compiles every TOML and term-source file under this
directory into the binary via `include_str!`, so adding a pattern or dictionary
is as simple as:

1. Drop the asset into the right subtree.
2. Wire a `shipped_pattern!` / `shipped_dictionary!` accessor in
   `src/shipped/{patterns,dictionaries}/<scope>.rs`.
3. Append the accessor to the sub-module's `all()`.

Each pattern and dictionary declares the languages and countries it applies to,
and the recognizer filters them per call so only the relevant rules run for a
given document.

## Layout

```
assets/
  patterns/
    world/                 jurisdiction-agnostic regex patterns
      contact/             email, phone, url
      credentials/         aws, github, stripe, generic api, private key
      finance/             credit card, iban, swift, btc, eth
      network/             ipv4, ipv6, mac
      personal/            date of birth, datetime
    us/                    US-jurisdiction patterns
      identity/            ssn, itin, drivers_license, passport, postal_code
      finance/             bank_routing, bank_account
      health/              npi, mbi, medical_license (DEA)
    uk/                    UK-jurisdiction patterns
      identity/            nhs, nino, driving_licence, passport
      contact/             postcode
      vehicle/             registration

  dictionaries/
    world/                 universal: brand names + codes
      finance/             cryptocurrencies (BTC, ETH, Bitcoin, …)
    en/                    English-language terms
      finance/             currencies (USD, US Dollar, EUR, …)
      personal/            languages, nationalities, religions

  walks/
    legal_form/            company names ending in a legal form
      walk.toml            the rule: name, label, direction
      forms.toml           legal forms by country, distinctive and ambiguous
      <lang>.toml          a language's context keywords, and the words that
                           shape a name: connectors, lead-in clauses, verbs,
                           roles
    institution/           institutions around a head word
      walk.toml, heads.toml, en.toml
    person_title/          a person's name after a title
      walk.toml, titles.toml, und.toml (any language)
```

Each pattern is a TOML file (`<name>.toml`). Each dictionary pairs a TOML
metadata sidecar with a term source: `.csv` for multi-column lists (term + alias
columns with per-column scores), `.txt` for one-per-line lists. A walk is a TOML
rule plus the anchors a name sits next to and the per-language words that shape
a name.

## Scoring conventions

Scores are baseline confidence; a rule's `[context]` (`elide_pattern::context`)
lifts them when its keywords appear nearby. The toolkit's default confidence
threshold is `0.35`; anything below needs context boost or an out-of-band hint
(CSV column header, JSON object key, HTML parent text) to clear it.

| Tier   | Score     | Use                                                                                    |
| ------ | --------- | -------------------------------------------------------------------------------------- |
| Strong | 0.95–0.98 | Branded credential headers (`AKIA…`, `-----BEGIN PRIVATE KEY-----`, `gh[pousr]_…`)     |
| Solid  | 0.4–0.5   | Format with a checksum or restrictive structure (IBAN, NHS, NPI, MBI, IPv4, MAC, IPv6) |
| Loose  | 0.3       | Brand-aware with weak structural specificity (credit_card, dictionaries)               |
| Weak   | 0.1       | Generic shape that _requires_ context to clear threshold (passport, postal_code, DoB)  |
| Trace  | 0.05      | Last-resort generic regex (bank_account `\b\d{8,17}\b`)                                |

The targets mirror Microsoft Presidio's deliberately-conservative baselines:
most of Presidio's predefined recognizers sit in 0.1–0.5 because the context
enhancer is expected to lift hits to 0.6+ when surrounding tokens match.

## Validators

A pattern variant can declare `validator = "<name>"` to drop matches that fail a
post-match structural check. Built-in names (resolved via
`ValidatorRegistry::builtin`):

- Universal: `luhn`, `iban`, `phone`, `date`, `crypto.btc`
- US: `us.ssn`, `us.aba_routing`, `us.npi`, `us.dea_number`, `us.postal_code`
- UK: `uk.nhs`, `uk.nino`, `uk.driving_licence`, `uk.vehicle_registration`

Each lives in `src/validators/` under the matching submodule.

## Spans

A match's span is the whole match, unless the regex defines a capture group
named `entity`: then only that group is reported, and the rest of the match is
context that must be present but is not part of the entity. This stands in for
lookaround, which the `regex` crate does not support:

```toml
# Require a token boundary after the address without including it.
regex = '\b(?P<entity>(?:[0-9A-Fa-f]{0,4}:){2,7}[0-9A-Fa-f]{1,4})(?:$|[^0-9A-Za-z:.%/_-])'
```

A validator sees only the group's text. Context outside the group is consumed,
so the next match starts after it. `world/network/ipv6.toml` uses this for both
a leading and a trailing boundary.

## Attribution

Many patterns + validators are ports of upstream Microsoft Presidio recognizers.
See [`PRESIDIO.md`](PRESIDIO.md) for the MIT-license attribution and the
upstream class references that each adapted TOML's leading comment links to.

The walks' anchors and word lists are converted from stella/anonymize; see
[`STELLA.md`](STELLA.md) for the Apache-2.0 attribution.
