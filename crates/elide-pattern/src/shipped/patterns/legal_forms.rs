//! Organization names anchored on a legal-form suffix: "Müller Bau GmbH",
//! "Bank of America Corp.", "Rossi S.p.A.".
//!
//! Each gazetteer under `assets/gazetteers/legal_forms/` lists one country's
//! suffixes, split into `distinctive` ones and `ambiguous` ones that are also
//! ordinary abbreviations (`AG`, `SA`, `LP`). [`organization`] merges them into
//! one [`Regex`] labelled `organization_name`, with a [`Variant`] per kind,
//! built from one template:
//!
//! - an optional lead-in word ("The", "From", "Der") that is matched but not
//!   reported, from `stop_words.txt`;
//! - capitalized words, initials, or numbers on one line, which may be joined
//!   by a connective ("Bank of America", "Procter & Gamble", "Van der Berg").
//!   The match crossing the fewest connectives wins, then the longest run of
//!   words after the last one: "Acme GmbH and Beta GmbH" is two names, while
//!   a suffix word inside a name stays part of it ("Arkansas Corporation
//!   Services, Inc."). Neither repetition is capped: a cap repeats the Unicode
//!   word classes once per word and multiplies the compiled size;
//! - an optional comma, then the suffix, then the end of the token.
//!
//! The `legal_form` validator drops a match whose name starts with a lead-in
//! word, so "The AG said" yields nothing. Ambiguous suffixes score below the
//! default threshold, so they surface only when a context keyword nearby lifts
//! them.
//!
//! The pattern is not limited to a country or language: a German company is
//! named "Siemens AG" in an English text too.

use elide_core::entity::builtins;
use elide_core::primitive::Confidence;
use serde::Deserialize;

use crate::recognition::Sourced;
use crate::validators::legal_form::stop_words;
use crate::{Regex, Variant};

/// One gazetteer per country.
const GAZETTEERS: [&str; 6] = [
    include_str!("../../../assets/gazetteers/legal_forms/us.toml"),
    include_str!("../../../assets/gazetteers/legal_forms/uk.toml"),
    include_str!("../../../assets/gazetteers/legal_forms/de.toml"),
    include_str!("../../../assets/gazetteers/legal_forms/fr.toml"),
    include_str!("../../../assets/gazetteers/legal_forms/it.toml"),
    include_str!("../../../assets/gazetteers/legal_forms/pl.toml"),
];

/// Score of a match on a distinctive suffix.
const DISTINCTIVE_SCORE: f32 = 0.5;

/// Score of a match on an ambiguous suffix: below the default threshold.
const AMBIGUOUS_SCORE: f32 = 0.3;

/// Whitespace within one line: a name never continues across a line break.
const SPACE: &str = r"[\p{Zs}\t]";

/// One word of a name: initials (`J.P.`), or a capitalized word or number.
const NAME_WORD: &str = r"(?:(?:\p{Lu}\.){1,4}|[\p{Lu}\p{N}][\p{L}\p{N}&'’-]*)";

/// A lowercase word that joins two words of a name.
const CONNECTIVE: &str = r"(?:&|and|und|of|de|di|da|del|der|den|van|von|du|des|la|le|y|e|et|i)";

/// One country's legal-form suffixes, as written in its asset.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Gazetteer {
    distinctive: Vec<String>,
    #[serde(default)]
    ambiguous: Vec<String>,
    #[serde(default)]
    context: Vec<String>,
}

/// Organization names ending in a legal form from any shipped gazetteer.
#[must_use]
pub fn organization() -> Regex {
    let mut distinctive = Vec::new();
    let mut ambiguous = Vec::new();
    let mut context = Vec::new();
    for source in GAZETTEERS {
        let gazetteer: Gazetteer =
            toml::from_str(source).expect("shipped legal-form gazetteer is well-formed");
        distinctive.extend(gazetteer.distinctive);
        ambiguous.extend(gazetteer.ambiguous);
        context.extend(gazetteer.context);
    }
    context.sort_unstable();
    context.dedup();

    Regex::builder()
        .with_name("legal-form")
        .with_labels(vec![builtins::ORGANIZATION_NAME.to_ref()])
        .with_context(Sourced {
            keywords: context,
            ..Sourced::default()
        })
        .with_variants(vec![
            variant(&distinctive, DISTINCTIVE_SCORE),
            variant(&ambiguous, AMBIGUOUS_SCORE),
        ])
        .build()
        .expect("legal-form rule builds")
}

/// Every legal-form pattern.
#[must_use]
pub fn all() -> Vec<Regex> {
    vec![organization()]
}

/// The variant matching a name that ends in one of `suffixes`.
fn variant(suffixes: &[String], score: f32) -> Variant {
    let mut lead_ins: Vec<&str> = stop_words().collect();
    lead_ins.sort_unstable();
    let lead_in = lead_ins
        .iter()
        .map(|word| regex::escape(word))
        .collect::<Vec<_>>()
        .join("|");
    let regex = format!(
        r"\b(?:(?i:{lead_in}){SPACE}+)?(?P<entity>(?:{words}{SPACE}+{CONNECTIVE}{SPACE}+)*?{words},?{SPACE}+(?:{suffixes}))(?:$|[^\p{{L}}\p{{N}}_])",
        words = format!("{NAME_WORD}(?:{SPACE}+{NAME_WORD})*"),
        suffixes = alternation(suffixes),
    );
    Variant::new(regex)
        .expect("legal-form regex compiles")
        .with_score(Confidence::new(score).expect("score is in range"))
        .with_validator("legal_form")
}

/// The suffixes as one alternation, longest first so `GmbH & Co. KG` wins over
/// `GmbH`. A space in a suffix matches any run of spaces on the line, or none
/// (`sp.z o.o.`, `GmbH&Co. KG`).
fn alternation(suffixes: &[String]) -> String {
    let mut sorted: Vec<&String> = suffixes.iter().collect();
    sorted.sort_by_key(|suffix| std::cmp::Reverse(suffix.chars().count()));
    sorted
        .iter()
        .map(|suffix| regex::escape(suffix).replace(' ', &format!("{SPACE}*")))
        .collect::<Vec<_>>()
        .join("|")
}
