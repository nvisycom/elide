//! [`LinguaDetector`]: a [`lingua`] language detector that reports
//! [`LanguageClaim`]s.
//!
//! [`lingua`]: https://crates.io/crates/lingua

use std::fmt;
use std::str::FromStr;

use elide_core::primitive::{Confidence, LanguageClaim, LanguageTag};
use lingua::{IsoCode639_1, Language, LanguageDetector, LanguageDetectorBuilder};

/// A `lingua` language detector over a fixed set of candidate languages.
///
/// Detects per region: mixed-language input yields one [`LanguageClaim`] per
/// region, each with its span; monolingual input yields one claim covering
/// the whole text.
pub(crate) struct LinguaDetector {
    inner: LanguageDetector,
}

impl LinguaDetector {
    /// A detector over the languages of `tags` that `lingua` was compiled
    /// with, matched by primary subtag; `None` when none of them is.
    pub(crate) fn for_languages<'a>(
        tags: impl IntoIterator<Item = &'a LanguageTag>,
    ) -> Option<Self> {
        let languages: Vec<Language> = tags.into_iter().filter_map(compiled_language).collect();
        if languages.is_empty() {
            return None;
        }
        Some(Self {
            inner: LanguageDetectorBuilder::from_languages(&languages).build(),
        })
    }

    /// A detector over every language `lingua` was compiled with.
    pub(crate) fn for_all_languages() -> Self {
        Self {
            inner: LanguageDetectorBuilder::from_all_languages().build(),
        }
    }

    /// The languages of `text`, one claim per region, each scored on its own
    /// region. Empty when `lingua` commits to no language.
    pub(crate) fn detect(&self, text: &str) -> Vec<LanguageClaim> {
        self.inner
            .detect_multiple_languages_of(text)
            .into_iter()
            .map(|region| {
                let span = region.start_index()..region.end_index();
                let confidence = self
                    .inner
                    .compute_language_confidence(&text[span.clone()], region.language());
                LanguageClaim::detected(
                    tag(region.language()),
                    Confidence::clamped(confidence as f32),
                )
                .with_span(span)
            })
            .collect()
    }
}

/// The `lingua` language of `tag`, matched by primary subtag, when `lingua`
/// was compiled with it.
pub(crate) fn compiled_language(tag: &LanguageTag) -> Option<Language> {
    IsoCode639_1::from_str(tag.primary_subtag())
        .ok()
        .map(|iso| Language::from_iso_code_639_1(&iso))
}

/// The language tag of a `lingua` language: its ISO 639-1 code, which is
/// always a valid two-letter primary subtag.
fn tag(language: Language) -> LanguageTag {
    language
        .iso_code_639_1()
        .to_string()
        .parse()
        .expect("every lingua ISO 639-1 code is a valid language tag")
}

impl fmt::Debug for LinguaDetector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LinguaDetector").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn english_only() -> LinguaDetector {
        let tags = ["en".parse().unwrap()];
        LinguaDetector::for_languages(&tags).expect("english is enabled")
    }

    #[test]
    fn every_compiled_language_has_a_tag() {
        for language in Language::all() {
            assert_eq!(
                tag(language).primary_subtag(),
                language.iso_code_639_1().to_string()
            );
        }
    }

    #[test]
    fn detects_english_sentence() {
        let detections = english_only().detect("The quick brown fox jumps over the lazy dog.");
        assert!(!detections.is_empty());
        assert_eq!(detections[0].language.primary_subtag(), "en");
        assert!(detections[0].span.is_some());
    }

    #[test]
    fn empty_input_returns_empty_vec() {
        assert!(english_only().detect("").is_empty());
    }

    #[test]
    fn no_compiled_language_builds_no_detector() {
        let tags = ["xx".parse().unwrap()];
        assert!(LinguaDetector::for_languages(&tags).is_none());
    }

    /// Each region of mixed text is scored on its own text, so every
    /// language is confident in its own region.
    #[cfg(any(feature = "common", feature = "all"))]
    #[test]
    fn each_region_is_scored_on_its_own_text() {
        let tags = ["en".parse().unwrap(), "de".parse().unwrap()];
        let detector = LinguaDetector::for_languages(&tags).unwrap();
        let text = "The quick brown fox jumps over the lazy dog and runs away. \
                    Der schnelle braune Fuchs springt über den faulen Hund und läuft weg.";
        let claims = detector.detect(text);
        let languages: Vec<&str> = claims.iter().map(|c| c.language.primary_subtag()).collect();
        assert_eq!(languages, ["en", "de"]);
        for claim in &claims {
            assert!(
                claim.confidence().get() > 0.5,
                "{} scored {}",
                claim.language.as_str(),
                claim.confidence().get()
            );
        }
    }
}
