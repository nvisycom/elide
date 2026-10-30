//! Built-in [`Walk`]s, embedded at compile time.
//!
//! - [`legal_form`]: company names ending in a legal form ("Müller Bau GmbH").
//! - [`institution`]: institutions around a head word ("U.S. Department of
//!   Justice").
//! - [`person_title`]: a person's name after a title ("Dr. Hans Müller").
//!
//! Each walk is a rule (`walk.toml`) plus its country and language files.
//! Their markers and words are converted from stella/anonymize; see
//! `assets/STELLA.md`.

use std::collections::BTreeMap;

use elide_core::primitive::{Confidence, LanguageTag};
use serde::Deserialize;

use crate::{Term, Walk, Words};

/// Score of a name next to a distinctive marker (`GmbH`, `Department`, `Dr.`).
const DISTINCTIVE_SCORE: f32 = 0.5;

/// Score of a name next to an ambiguous one (`AG`, `Office`, `Judge`): below
/// the default threshold, so it needs a context keyword nearby.
const AMBIGUOUS_SCORE: f32 = 0.3;

/// A walk asset, by its path in `assets/walks/` without the `.toml`.
macro_rules! asset {
    ($path:literal) => {
        include_str!(concat!("../../assets/walks/", $path, ".toml"))
    };
}

/// The legal-form walk's country files, by country code.
const LEGAL_FORM_COUNTRIES: &[(&str, &str)] = &[
    ("ad", asset!("legal_form/countries/ad")),
    ("ae", asset!("legal_form/countries/ae")),
    ("al", asset!("legal_form/countries/al")),
    ("am", asset!("legal_form/countries/am")),
    ("ar", asset!("legal_form/countries/ar")),
    ("at", asset!("legal_form/countries/at")),
    ("au", asset!("legal_form/countries/au")),
    ("az", asset!("legal_form/countries/az")),
    ("ba", asset!("legal_form/countries/ba")),
    ("bb", asset!("legal_form/countries/bb")),
    ("be", asset!("legal_form/countries/be")),
    ("bg", asset!("legal_form/countries/bg")),
    ("bh", asset!("legal_form/countries/bh")),
    ("br", asset!("legal_form/countries/br")),
    ("ca", asset!("legal_form/countries/ca")),
    ("ch", asset!("legal_form/countries/ch")),
    ("cl", asset!("legal_form/countries/cl")),
    ("co", asset!("legal_form/countries/co")),
    ("cr", asset!("legal_form/countries/cr")),
    ("cy", asset!("legal_form/countries/cy")),
    ("cz", asset!("legal_form/countries/cz")),
    ("de", asset!("legal_form/countries/de")),
    ("dk", asset!("legal_form/countries/dk")),
    ("do", asset!("legal_form/countries/do")),
    ("ee", asset!("legal_form/countries/ee")),
    ("eg", asset!("legal_form/countries/eg")),
    ("es", asset!("legal_form/countries/es")),
    ("fi", asset!("legal_form/countries/fi")),
    ("fr", asset!("legal_form/countries/fr")),
    ("gb", asset!("legal_form/countries/gb")),
    ("ge", asset!("legal_form/countries/ge")),
    ("gh", asset!("legal_form/countries/gh")),
    ("gr", asset!("legal_form/countries/gr")),
    ("hk", asset!("legal_form/countries/hk")),
    ("hr", asset!("legal_form/countries/hr")),
    ("hu", asset!("legal_form/countries/hu")),
    ("id", asset!("legal_form/countries/id")),
    ("ie", asset!("legal_form/countries/ie")),
    ("il", asset!("legal_form/countries/il")),
    ("in", asset!("legal_form/countries/in")),
    ("is", asset!("legal_form/countries/is")),
    ("it", asset!("legal_form/countries/it")),
    ("jm", asset!("legal_form/countries/jm")),
    ("jp", asset!("legal_form/countries/jp")),
    ("ke", asset!("legal_form/countries/ke")),
    ("kr", asset!("legal_form/countries/kr")),
    ("kw", asset!("legal_form/countries/kw")),
    ("kz", asset!("legal_form/countries/kz")),
    ("li", asset!("legal_form/countries/li")),
    ("lt", asset!("legal_form/countries/lt")),
    ("lu", asset!("legal_form/countries/lu")),
    ("lv", asset!("legal_form/countries/lv")),
    ("ma", asset!("legal_form/countries/ma")),
    ("mc", asset!("legal_form/countries/mc")),
    ("md", asset!("legal_form/countries/md")),
    ("me", asset!("legal_form/countries/me")),
    ("mk", asset!("legal_form/countries/mk")),
    ("mt", asset!("legal_form/countries/mt")),
    ("mx", asset!("legal_form/countries/mx")),
    ("my", asset!("legal_form/countries/my")),
    ("ng", asset!("legal_form/countries/ng")),
    ("nl", asset!("legal_form/countries/nl")),
    ("no", asset!("legal_form/countries/no")),
    ("nz", asset!("legal_form/countries/nz")),
    ("om", asset!("legal_form/countries/om")),
    ("pa", asset!("legal_form/countries/pa")),
    ("pe", asset!("legal_form/countries/pe")),
    ("ph", asset!("legal_form/countries/ph")),
    ("pl", asset!("legal_form/countries/pl")),
    ("pr", asset!("legal_form/countries/pr")),
    ("pt", asset!("legal_form/countries/pt")),
    ("qa", asset!("legal_form/countries/qa")),
    ("ro", asset!("legal_form/countries/ro")),
    ("rs", asset!("legal_form/countries/rs")),
    ("sa", asset!("legal_form/countries/sa")),
    ("se", asset!("legal_form/countries/se")),
    ("sg", asset!("legal_form/countries/sg")),
    ("si", asset!("legal_form/countries/si")),
    ("sk", asset!("legal_form/countries/sk")),
    ("sm", asset!("legal_form/countries/sm")),
    ("th", asset!("legal_form/countries/th")),
    ("tn", asset!("legal_form/countries/tn")),
    ("tr", asset!("legal_form/countries/tr")),
    ("tt", asset!("legal_form/countries/tt")),
    ("us", asset!("legal_form/countries/us")),
    ("uy", asset!("legal_form/countries/uy")),
    ("uz", asset!("legal_form/countries/uz")),
    ("ve", asset!("legal_form/countries/ve")),
    ("vn", asset!("legal_form/countries/vn")),
    ("xk", asset!("legal_form/countries/xk")),
    ("za", asset!("legal_form/countries/za")),
];

/// The legal-form walk's language files, by language tag.
const LEGAL_FORM_LANGUAGES: &[(&str, &str)] = &[
    ("cs", asset!("legal_form/languages/cs")),
    ("de", asset!("legal_form/languages/de")),
    ("en", asset!("legal_form/languages/en")),
    ("es", asset!("legal_form/languages/es")),
    ("fr", asset!("legal_form/languages/fr")),
    ("hu", asset!("legal_form/languages/hu")),
    ("it", asset!("legal_form/languages/it")),
    ("nl", asset!("legal_form/languages/nl")),
    ("pl", asset!("legal_form/languages/pl")),
    ("pt-BR", asset!("legal_form/languages/pt-br")),
    ("ro", asset!("legal_form/languages/ro")),
    ("sk", asset!("legal_form/languages/sk")),
    ("sv", asset!("legal_form/languages/sv")),
];

/// The institution walk's language files, by language tag.
const INSTITUTION_LANGUAGES: &[(&str, &str)] = &[("en", asset!("institution/languages/en"))];

/// The person-title walk's language files, by language tag.
const PERSON_TITLE_LANGUAGES: &[(&str, &str)] = &[
    ("cs", asset!("person_title/languages/cs")),
    ("de", asset!("person_title/languages/de")),
    ("en", asset!("person_title/languages/en")),
    ("es", asset!("person_title/languages/es")),
    ("fr", asset!("person_title/languages/fr")),
    ("it", asset!("person_title/languages/it")),
    ("pt", asset!("person_title/languages/pt")),
    ("ro", asset!("person_title/languages/ro")),
    ("sk", asset!("person_title/languages/sk")),
];

/// A walk's markers, split by how much a name next to one is worth.
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Markers {
    #[serde(default)]
    distinctive: Vec<String>,
    #[serde(default)]
    ambiguous: Vec<String>,
}

/// A country file: the markers the country defines.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Country {
    markers: Markers,
}

/// A language file: the keywords that raise a name's confidence, the markers
/// the language uses, and the words that shape a name.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Language {
    #[serde(default)]
    keywords: Vec<String>,
    #[serde(default)]
    markers: Markers,
    words: Option<Words>,
}

/// Company names ending in a legal form, in any language.
#[must_use]
pub fn legal_form() -> Walk {
    walk(
        asset!("legal_form/walk"),
        LEGAL_FORM_COUNTRIES,
        LEGAL_FORM_LANGUAGES,
    )
}

/// Institutions named around a head word, in English.
#[must_use]
pub fn institution() -> Walk {
    walk(asset!("institution/walk"), &[], INSTITUTION_LANGUAGES)
}

/// A person's name after a title, in any language. The title is not part of
/// the name.
#[must_use]
pub fn person_title() -> Walk {
    walk(asset!("person_title/walk"), &[], PERSON_TITLE_LANGUAGES)
}

/// Every built-in walk.
#[must_use]
pub fn all() -> Vec<Walk> {
    vec![legal_form(), institution(), person_title()]
}

/// The walk in `rule`, with the markers of its `countries` and `languages`
/// files and the keywords and words of its `languages` files.
///
/// A marker in several files (`GmbH` in DE, AT, and CH; `Dr.` in most
/// languages) is one marker, ambiguous if any file says so.
fn walk(rule: &str, countries: &[(&str, &str)], languages: &[(&str, &str)]) -> Walk {
    let mut walk = Walk::from_toml(rule).expect("shipped walk is well-formed");
    let mut scores: BTreeMap<String, f32> = BTreeMap::new();
    let mut add = |markers: Markers| {
        for marker in markers.distinctive {
            scores.entry(marker).or_insert(DISTINCTIVE_SCORE);
        }
        for marker in markers.ambiguous {
            scores.insert(marker, AMBIGUOUS_SCORE);
        }
    };

    for &(_, source) in countries {
        let file: Country = toml::from_str(source).expect("shipped walk country is well-formed");
        add(file.markers);
    }
    for &(tag, source) in languages {
        let language: LanguageTag = tag.parse().expect("shipped language tag parses");
        let file: Language = toml::from_str(source).expect("shipped walk language is well-formed");
        add(file.markers);
        if !file.keywords.is_empty() {
            walk.context
                .languages
                .insert(language.clone(), file.keywords);
        }
        if let Some(words) = file.words {
            walk.words.insert(language, words);
        }
    }

    walk.markers = scores
        .into_iter()
        .map(|(marker, score)| Term::new(marker).with_score(Confidence::clamped(score)))
        .collect();
    walk
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::fs;
    use std::path::Path;

    use super::*;

    /// The `.toml` file stems in `dir`, under `assets/walks/`.
    fn stems(dir: &str) -> BTreeSet<String> {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("assets/walks")
            .join(dir);
        fs::read_dir(dir)
            .expect("asset folder reads")
            .map(|entry| entry.expect("asset entry reads").path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "toml"))
            .map(|path| {
                path.file_stem()
                    .expect("file stem")
                    .to_string_lossy()
                    .into_owned()
            })
            .collect()
    }

    #[test]
    fn every_country_and_language_file_is_loaded() {
        for (dir, files) in [
            ("legal_form/countries", LEGAL_FORM_COUNTRIES),
            ("legal_form/languages", LEGAL_FORM_LANGUAGES),
            ("institution/languages", INSTITUTION_LANGUAGES),
            ("person_title/languages", PERSON_TITLE_LANGUAGES),
        ] {
            let loaded: BTreeSet<String> =
                files.iter().map(|(key, _)| key.to_lowercase()).collect();
            assert_eq!(loaded, stems(dir), "in {dir}");
        }
    }
}
