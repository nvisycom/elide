//! Built-in [`Walk`]s, embedded at compile time.
//!
//! - [`legal_form`]: company names ending in a legal form ("Müller Bau GmbH").
//! - [`institution`]: institutions around a head word ("U.S. Department of
//!   Justice").
//! - [`person_title`]: a person's name after a title ("Dr. Hans Müller").
//!
//! Their anchors and words are converted from stella/anonymize; see
//! `assets/STELLA.md`.

use std::collections::{BTreeMap, HashMap};

use elide_core::primitive::{Confidence, LanguageTag};
use serde::Deserialize;

use crate::{Term, Walk, Words};

/// Score of a name next to a distinctive anchor (`GmbH`, `Department`, `Dr.`).
const DISTINCTIVE_SCORE: f32 = 0.5;

/// Score of a name next to an ambiguous one (`AG`, `Office`, `Judge`): below
/// the default threshold, so it needs a context keyword nearby.
const AMBIGUOUS_SCORE: f32 = 0.3;

/// The legal-form walk's language files, in `assets/walks/legal_form/`.
const LEGAL_FORM_LANGUAGES: [(&str, &str); 13] = [
    ("cs", include_str!("../../assets/walks/legal_form/cs.toml")),
    ("de", include_str!("../../assets/walks/legal_form/de.toml")),
    ("en", include_str!("../../assets/walks/legal_form/en.toml")),
    ("es", include_str!("../../assets/walks/legal_form/es.toml")),
    ("fr", include_str!("../../assets/walks/legal_form/fr.toml")),
    ("hu", include_str!("../../assets/walks/legal_form/hu.toml")),
    ("it", include_str!("../../assets/walks/legal_form/it.toml")),
    ("nl", include_str!("../../assets/walks/legal_form/nl.toml")),
    ("pl", include_str!("../../assets/walks/legal_form/pl.toml")),
    (
        "pt-BR",
        include_str!("../../assets/walks/legal_form/pt-br.toml"),
    ),
    ("ro", include_str!("../../assets/walks/legal_form/ro.toml")),
    ("sk", include_str!("../../assets/walks/legal_form/sk.toml")),
    ("sv", include_str!("../../assets/walks/legal_form/sv.toml")),
];

/// A walk's anchors, split by how much a name next to one is worth.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Anchors {
    #[serde(default)]
    distinctive: Vec<String>,
    #[serde(default)]
    ambiguous: Vec<String>,
}

/// One language's file: the keywords that raise a name's confidence, and the
/// words that shape a name.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Language {
    #[serde(default)]
    keywords: Vec<String>,
    words: Words,
}

/// Company names ending in a legal form, in any language.
#[must_use]
pub fn legal_form() -> Walk {
    // A form several countries share (`GmbH` in DE, AT, and CH) is one anchor,
    // ambiguous if any country lists it so.
    let countries: HashMap<String, Anchors> =
        toml::from_str(include_str!("../../assets/walks/legal_form/forms.toml"))
            .expect("shipped legal forms are well-formed");
    let mut walk = walk(
        include_str!("../../assets/walks/legal_form/walk.toml"),
        countries.into_values(),
    );
    for (tag, source) in LEGAL_FORM_LANGUAGES {
        add_language(&mut walk, tag, source);
    }
    walk
}

/// Institutions named around a head word, in English.
#[must_use]
pub fn institution() -> Walk {
    let mut walk = walk(
        include_str!("../../assets/walks/institution/walk.toml"),
        [anchors(include_str!(
            "../../assets/walks/institution/heads.toml"
        ))],
    );
    add_language(
        &mut walk,
        "en",
        include_str!("../../assets/walks/institution/en.toml"),
    );
    walk
}

/// A person's name after a title, in any language. The title is not part of
/// the name.
#[must_use]
pub fn person_title() -> Walk {
    let mut walk = walk(
        include_str!("../../assets/walks/person_title/walk.toml"),
        [anchors(include_str!(
            "../../assets/walks/person_title/titles.toml"
        ))],
    );
    add_language(
        &mut walk,
        "und",
        include_str!("../../assets/walks/person_title/und.toml"),
    );
    walk
}

/// Every built-in walk.
#[must_use]
pub fn all() -> Vec<Walk> {
    vec![legal_form(), institution(), person_title()]
}

/// The walk in `source`, anchored on every anchor in `lists`. An anchor in
/// several lists is ambiguous if any list says so.
fn walk(source: &str, lists: impl IntoIterator<Item = Anchors>) -> Walk {
    let mut walk = Walk::from_toml(source).expect("shipped walk is well-formed");
    let mut scores: BTreeMap<String, f32> = BTreeMap::new();
    for list in lists {
        for anchor in list.distinctive {
            scores.entry(anchor).or_insert(DISTINCTIVE_SCORE);
        }
        for anchor in list.ambiguous {
            scores.insert(anchor, AMBIGUOUS_SCORE);
        }
    }
    walk.anchors = scores
        .into_iter()
        .map(|(anchor, score)| Term::new(anchor).with_score(Confidence::clamped(score)))
        .collect();
    walk
}

fn anchors(source: &str) -> Anchors {
    toml::from_str(source).expect("shipped walk anchors are well-formed")
}

/// Add the language file `source` for `tag` to `walk`: its keywords to the
/// walk's context, its words to the walk's words.
fn add_language(walk: &mut Walk, tag: &str, source: &str) {
    let language: LanguageTag = tag.parse().expect("shipped language tag parses");
    let file: Language = toml::from_str(source).expect("shipped walk language is well-formed");
    if !file.keywords.is_empty() {
        walk.context
            .languages
            .insert(language.clone(), file.keywords);
    }
    walk.words.insert(language, file.words);
}
