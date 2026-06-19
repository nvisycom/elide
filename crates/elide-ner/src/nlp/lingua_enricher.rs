//! [`LinguaEnricher`]: a language-detection [`Enricher`] backed by the
//! [`lingua`] crate.
//!
//! Runs language detection over the input text and adds the detected
//! languages to the call. A pattern-only pipeline that wants its rules
//! scoped to the document's language registers one of these ahead of its
//! recognizers; the context enhancer and any language-aware recognizer
//! then read the call's languages back from the input.
//!
//! When the caller has already asserted a language on the input,
//! detection is skipped: the assertion is authoritative.
//!
//! [`lingua`]: https://crates.io/crates/lingua

use elide_core::Result;
use elide_core::modality::text::{Text, TextData};
use elide_core::primitive::LanguageTag;
use elide_core::recognition::{Enricher, RecognizerContext};

use super::lingua_detector::LinguaDetector;

/// Lingua-backed language-detection enricher.
///
/// Stateless: every call builds a fresh [`LinguaDetector`] for the
/// configured language scope (the candidate set passed at construction,
/// or every language when unrestricted). The scope is fixed at
/// construction; pipelines that need different scopes per call hold
/// multiple enrichers.
#[derive(Debug, Clone)]
pub struct LinguaEnricher {
    candidates: Vec<LanguageTag>,
}

impl LinguaEnricher {
    /// An enricher that considers every language compiled into the
    /// `lingua` crate's feature set.
    #[must_use]
    pub fn unrestricted() -> Self {
        Self {
            candidates: Vec::new(),
        }
    }

    /// An enricher restricted to `candidates`. Tags lingua doesn't
    /// recognise are silently skipped at detector-build time; an empty
    /// input is equivalent to [`unrestricted`](Self::unrestricted).
    #[must_use]
    pub fn with_candidates(candidates: impl IntoIterator<Item = LanguageTag>) -> Self {
        Self {
            candidates: candidates.into_iter().collect(),
        }
    }

    fn detector(&self) -> LinguaDetector {
        if self.candidates.is_empty() {
            LinguaDetector::for_all_languages()
        } else {
            LinguaDetector::for_languages(&self.candidates)
                .unwrap_or_else(LinguaDetector::for_all_languages)
        }
    }
}

impl Default for LinguaEnricher {
    fn default() -> Self {
        Self::unrestricted()
    }
}

impl Enricher<Text> for LinguaEnricher {
    async fn enrich(&self, data: &TextData, ctx: &mut RecognizerContext<'_, Text>) -> Result<()> {
        // A caller-asserted language is authoritative; skip detection.
        if ctx.has_asserted_language() {
            return Ok(());
        }
        for detection in self.detector().detect(data.text.as_str())? {
            ctx.detect_language(detection);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use elide_core::modality::text::TextData;
    use elide_core::primitive::Language;
    use elide_core::recognition::Scope;

    use super::*;

    #[tokio::test]
    async fn detects_english_onto_input() {
        let data = TextData::new("The quick brown fox jumps over the lazy dog.");
        let scope = Scope::new();
        let mut ctx = RecognizerContext::new(&scope);
        LinguaEnricher::unrestricted()
            .enrich(&data, &mut ctx)
            .await
            .unwrap();
        assert_eq!(ctx.primary_language().unwrap().primary_language(), "en");
    }

    #[tokio::test]
    async fn asserted_language_skips_detection() {
        let de: LanguageTag = "de".parse().unwrap();
        let data = TextData::new("The quick brown fox");
        let scope = Scope::new().with_language(Language::asserted(de));
        let mut ctx = RecognizerContext::new(&scope);
        LinguaEnricher::unrestricted()
            .enrich(&data, &mut ctx)
            .await
            .unwrap();
        // Only the asserted German remains; English was never detected.
        assert_eq!(ctx.ranked_languages().len(), 1);
        assert_eq!(ctx.primary_language().unwrap().primary_language(), "de");
    }
}
