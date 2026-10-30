# stella/anonymize attribution

The walks under `walks/` follow [stella/anonymize](https://github.com/stella/anonymize)'s
legal-form detector, and their data is converted from it:

- `walks/legal_form/forms.toml`: the Latin-script forms of
  `packages/data/config/legal-forms.json`, by country, and the core suffix list
  of `crates/anonymize-adapter-contract/src/assemble/legal_forms.rs`, each also
  in capitals, split into distinctive and ambiguous forms.
- `walks/legal_form/<lang>.toml`: the `[words]` tables, from
  `packages/data/config/`: `legal-form-lowercase-bridge.json`,
  `in-name-connectors.json`, `legal-form-rule-words.json`,
  `legal-form-leading-clauses.json`, `sentence-verb-indicators.json`,
  `legal-role-heads.*.json`, `connector-prose-heads.json`,
  `clause-noun-heads.json`, and `structural-single-cap-prefixes.json`.
- `walks/institution/`: `institutional-organization-heads.json`,
  `institutional-organization-complement-connectors.json`,
  `institutional-organization-generic-name-words.json`, and
  `institutional-organization-prefix-generic-name-words.json`, with the English
  lead-in clauses and verbs above.
- `walks/person_title/`: `honorifics.json`, `names-title-tokens.json`, and the
  Arabic connectors of `name-corpus-particles.json`.

```
anonymize
Copyright 2026 stella labs, s.r.o.

This product includes software developed at stella labs, s.r.o.
(https://stll.app/).

Licensed under the Apache License, Version 2.0 (the "License"); you may
not use this product except in compliance with the License. You may
obtain a copy of the License at https://www.apache.org/licenses/LICENSE-2.0.
```
