//! The language-detection enricher building block.
//!
//! Unlike OCR and STT, language detection is pure Rust
//! ([`LinguaEnricher`](elide::enrichment::lingua::LinguaEnricher)) and needs no
//! JavaScript model, so [`Enricher::language`](super::Enricher::language) takes
//! no callback.

use elide::enrichment::lingua::LinguaEnricher;
use elide::primitive::LanguageTag;
use serde::Deserialize;
use tsify::{Ts, Tsify};

use crate::error::{ElideError, ElideErrorKind};

/// The languages a language-detection enricher considers: a named preset or an
/// explicit list of BCP-47 tags.
///
/// `Tsify` generates `type LanguageSet = LanguagePreset | string[]`, so JS passes
/// either a preset string (`"common"`) or an array (`["en", "tr", "el"]`).
/// Restricting the set raises precision and speed; the wider the set, the more
/// languages can be detected.
#[derive(Deserialize, Tsify)]
#[serde(rename_all = "camelCase", untagged)]
pub enum LanguageSet {
    /// A named preset.
    Preset(LanguagePreset),
    /// An explicit set of BCP-47 tags, e.g. `["en", "tr", "el"]`.
    Tags(Vec<String>),
}

/// A named language preset, from narrowest to widest.
#[derive(Deserialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub enum LanguagePreset {
    /// English only: the fastest, for English-dominant input.
    English,
    /// A curated set of widely-used languages, the practical default.
    Common,
    /// Every language the detector supports, the widest and slowest.
    All,
}

/// The curated set behind [`LanguagePreset::Common`]: the most widely used
/// languages, as BCP-47 tags.
const COMMON: &[&str] = &[
    "en", "es", "fr", "de", "it", "pt", "nl", "ru", "zh", "ja", "ar", "hi",
];

/// Build the language-detection enricher over `languages`.
pub(super) fn build_language(languages: Ts<LanguageSet>) -> Result<LinguaEnricher, ElideError> {
    let languages = languages.to_rust().map_err(|e| {
        ElideError::new(
            ElideErrorKind::Configuration,
            format!("invalid language set: {e}"),
        )
    })?;
    Ok(match languages {
        LanguageSet::Preset(LanguagePreset::All) => LinguaEnricher::unrestricted(),
        LanguageSet::Preset(LanguagePreset::English) => enricher_for(&["en"])?,
        LanguageSet::Preset(LanguagePreset::Common) => enricher_for(COMMON)?,
        LanguageSet::Tags(tags) if tags.is_empty() => LinguaEnricher::unrestricted(),
        LanguageSet::Tags(tags) => {
            let refs: Vec<&str> = tags.iter().map(String::as_str).collect();
            enricher_for(&refs)?
        }
    })
}

/// A [`LinguaEnricher`] restricted to `tags`, parsing each BCP-47 tag.
fn enricher_for(tags: &[&str]) -> Result<LinguaEnricher, ElideError> {
    let candidates = tags
        .iter()
        .map(|tag| LanguageTag::parse(*tag))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| {
            ElideError::new(
                ElideErrorKind::Configuration,
                format!("invalid BCP-47 language tag: {e}"),
            )
        })?;
    Ok(LinguaEnricher::with_candidates(candidates))
}
