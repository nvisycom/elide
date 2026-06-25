//! [`LinguaDetector`]: thin wrapper around the
//! [`lingua`] crate's
//! `LanguageDetector`.
//!
//! Owns the lingua detector and exposes one method,
//! [`detect`], that returns a
//! [`Vec<Language>`] in our ontology shape. Used by
//! [`LinguaEnricher`].
//!
//! Construction takes either a candidate-language set or "all
//! languages compiled into the lingua feature set"; the latter
//! is the unrestricted fallback.
//!
//! [`lingua`]: https://crates.io/crates/lingua
//! [`detect`]: LinguaDetector::detect
//! [`LinguaEnricher`]: crate::LinguaEnricher

use std::collections::HashSet;
use std::fmt;
use std::str::FromStr;
use std::sync::{Mutex, OnceLock};

use elide_core::Result;
use elide_core::primitive::{Confidence, Language, LanguageProvenance, LanguageSpan, LanguageTag};
use lingua::{
    IsoCode639_1, Language as LinguaLanguage, LanguageDetector as LinguaInner,
    LanguageDetectorBuilder,
};

/// Lingua-backed language detector.
///
/// Detects per-region languages: for mixed-language input,
/// returns one [`Language`] per detected region with a
/// populated [`LanguageSpan`]. Monolingual input returns a single
/// detection covering the whole text.
///
/// Internal helper for [`LinguaEnricher`], which builds a fresh detector
/// per call; not part of the public API.
///
/// [`LinguaEnricher`]: crate::LinguaEnricher
pub(crate) struct LinguaDetector {
    inner: LinguaInner,
}

impl LinguaDetector {
    /// Construct a detector restricted to `tags`. Unrecognised tags (no
    /// matching ISO 639-1 primary subtag in lingua) are silently skipped.
    /// Returns `None` when no tag matched; [`LinguaEnricher`] falls back to
    /// [`for_all_languages`] in that case.
    ///
    /// [`LinguaEnricher`]: crate::LinguaEnricher
    ///
    /// [`for_all_languages`]: Self::for_all_languages
    pub(crate) fn for_languages(tags: &[LanguageTag]) -> Option<Self> {
        let langs = tags_to_languages(tags);
        if langs.is_empty() {
            return None;
        }
        Some(Self {
            inner: LanguageDetectorBuilder::from_languages(&langs).build(),
        })
    }

    /// Construct a detector considering every language compiled
    /// into the `lingua` crate's feature set.
    pub(crate) fn for_all_languages() -> Self {
        Self {
            inner: LanguageDetectorBuilder::from_all_languages().build(),
        }
    }

    /// Run detection on `text`.
    ///
    /// Empty for ambiguous inputs (lingua refused to commit), one
    /// or more entries otherwise. Each entry has a populated
    /// [`LanguageSpan`] so mixed-language input can be attributed
    /// region-by-region.
    pub fn detect(&self, text: &str) -> Result<Vec<Language>> {
        let detections = self
            .inner
            .detect_multiple_languages_of(text)
            .into_iter()
            .filter_map(|result| {
                let language = lingua_to_tag(result.language())?;
                let raw_confidence = self
                    .inner
                    .compute_language_confidence(text, result.language());
                let confidence = Confidence::new(raw_confidence.clamp(0.0, 1.0) as f32);
                Some(Language {
                    language,
                    confidence,
                    provenance: LanguageProvenance::Detected,
                    span: Some(LanguageSpan {
                        start: result.start_index(),
                        end: result.end_index(),
                    }),
                })
            })
            .collect();
        Ok(detections)
    }
}

fn lingua_to_tag(lang: LinguaLanguage) -> Option<LanguageTag> {
    let iso = lang.iso_code_639_1().to_string();
    match iso.parse() {
        Ok(tag) => Some(tag),
        Err(e) => {
            warn_once_unmappable(&iso, &e.to_string());
            None
        }
    }
}

/// Cache of ISO codes we've already logged an "unmappable" warning
/// for, so a hot detection loop doesn't spam the log with the same
/// failure once per call. Lingua's code set is finite and fixed;
/// real failures here are deterministic.
fn warn_once_unmappable(iso: &str, error: &str) {
    static SEEN: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    let seen = SEEN.get_or_init(|| Mutex::new(HashSet::new()));
    let mut guard = match seen.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    if guard.insert(iso.to_owned()) {
        tracing::warn!(
            target: "elide_lingua",
            iso_code = %iso,
            error = %error,
            "lingua ISO 639-1 code did not parse as a BCP-47 LanguageTag (logged once per process)",
        );
    }
}

fn tags_to_languages(tags: &[LanguageTag]) -> Vec<LinguaLanguage> {
    tags.iter()
        .filter_map(|t| IsoCode639_1::from_str(t.primary_language()).ok())
        .map(|iso| LinguaLanguage::from_iso_code_639_1(&iso))
        .collect()
}

impl fmt::Debug for LinguaDetector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LinguaDetector").finish()
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
    fn detects_english_sentence() {
        let det = english_only();
        let detections = det
            .detect("The quick brown fox jumps over the lazy dog.")
            .unwrap();
        assert!(!detections.is_empty());
        let first = &detections[0];
        assert_eq!(first.language.primary_language(), "en");
        assert!(first.span.is_some());
    }

    #[test]
    fn empty_input_returns_empty_vec() {
        let det = english_only();
        assert!(det.detect("").unwrap().is_empty());
    }

    #[test]
    fn rejects_construction_with_no_recognised_languages() {
        let tags = ["xx".parse().unwrap()];
        assert!(LinguaDetector::for_languages(&tags).is_none());
    }
}
