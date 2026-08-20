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
use elide_core::modality::TextRecognizable;
use elide_core::primitive::LanguageTag;
use elide_core::recognition::{Enricher, Enrichment, RecognizerContext, RecognizerId};

use crate::lingua_detector::LinguaDetector;

/// Lingua-backed language-detection enricher.
///
/// Stateless: every call builds a fresh detector for the
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
    /// input is equivalent to [`unrestricted`].
    ///
    /// [`unrestricted`]: Self::unrestricted
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

/// Detects language over any [`TextRecognizable`] modality — `Text` and
/// `Tabular` project their payload as text identically, and an audio transcript
/// is read from the call's artifacts — so the same enricher scopes a text,
/// tabular (CSV/XLSX), or transcript pipeline to its detected language.
#[async_trait::async_trait]
impl<M: TextRecognizable> Enricher<M> for LinguaEnricher {
    fn id(&self) -> RecognizerId {
        RecognizerId::new("elide-lingua", env!("CARGO_PKG_VERSION"))
    }

    async fn enrich(
        &self,
        data: &M::Data,
        ctx: &mut RecognizerContext<'_, M>,
    ) -> Result<Enrichment> {
        // A caller-asserted language is authoritative; skip detection.
        if ctx.has_asserted_language() {
            return Ok(Enrichment::none());
        }
        // Detect into an owned list first so the immutable borrow of the payload
        // text ends before `detect_language` takes `&mut ctx`.
        let detections = self.detector().detect(M::as_text(data, &ctx.artifacts))?;
        for detection in detections {
            ctx.detect_language(detection);
        }
        // Language detection is pure-CPU: no model tokens to report.
        Ok(Enrichment::none())
    }
}

#[cfg(test)]
mod tests {
    use elide_core::modality::tabular::Tabular;
    use elide_core::modality::text::{Text, TextData};
    use elide_core::primitive::Language;
    use elide_core::recognition::Scope;

    use super::*;

    #[tokio::test]
    async fn detects_english_onto_input() {
        let data = TextData::new("The quick brown fox jumps over the lazy dog.");
        let scope = Scope::new();
        let mut ctx = RecognizerContext::<Text>::new(&scope);
        LinguaEnricher::unrestricted()
            .enrich(&data, &mut ctx)
            .await
            .unwrap();
        assert_eq!(ctx.primary_language().unwrap().primary_language(), "en");
    }

    #[tokio::test]
    async fn detects_onto_a_tabular_payload() {
        // Tabular cells are text, so the same enricher scopes a CSV/XLSX
        // pipeline to its detected language.
        let data = TextData::new("The quick brown fox jumps over the lazy dog.");
        let scope = Scope::new();
        let mut ctx = RecognizerContext::<Tabular>::new(&scope);
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
        let mut ctx = RecognizerContext::<Text>::new(&scope);
        LinguaEnricher::unrestricted()
            .enrich(&data, &mut ctx)
            .await
            .unwrap();
        // Only the asserted German remains; English was never detected.
        assert_eq!(ctx.ranked_languages().len(), 1);
        assert_eq!(ctx.primary_language().unwrap().primary_language(), "de");
    }
}
