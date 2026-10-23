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
use elide_core::enrichment::Enricher;
use elide_core::modality::TextRecognizable;
use elide_core::primitive::{ComponentId, LanguageTag};
use elide_core::recognition::{Context, Subject};

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

    /// A detector scoped to the caller's `asserted` languages, or `None` when
    /// detection should not run.
    ///
    /// For a nonempty **asserted** set the detector is restricted to it, and if
    /// none of those tags is a language `lingua` supports the result is `None`
    /// — detection is skipped rather than widened, because falling back to all
    /// languages would stamp a document with a language the caller never
    /// declared. For the enricher's own configured candidates (or none), the
    /// all-language fallback stands: that path wants a best-effort guess.
    fn detector_for(&self, asserted: &[LanguageTag]) -> Option<LinguaDetector> {
        if !asserted.is_empty() {
            // Asserted: restrict, and do not widen past the assertion.
            return LinguaDetector::for_languages(asserted);
        }
        // Configured candidates, else every language — both best-effort.
        Some(if self.candidates.is_empty() {
            LinguaDetector::for_all_languages()
        } else {
            LinguaDetector::for_languages(&self.candidates)
                .unwrap_or_else(LinguaDetector::for_all_languages)
        })
    }
}

impl Default for LinguaEnricher {
    fn default() -> Self {
        Self::unrestricted()
    }
}

/// Detects language over any [`TextRecognizable`] modality, `Text` and
/// `Tabular` project their payload as text identically, and an audio transcript
/// is read from the call's artifacts, so the same enricher scopes a text,
/// tabular (CSV/XLSX), or transcript pipeline to its detected language.
#[async_trait::async_trait]
impl<M: TextRecognizable> Enricher<M> for LinguaEnricher {
    fn id(&self) -> ComponentId {
        ComponentId::new("elide-lingua", env!("CARGO_PKG_VERSION"))
    }

    async fn enrich(&self, subject: &mut Subject<M>, ctx: &Context<'_, M>) -> Result<()> {
        // One asserted language is authoritative and monolingual: skip detection
        // and let every entity be attributed to it. Two or more means a mixed
        // document, so detection still runs, but restricted to the asserted set,
        // so each entity is attributed to the asserted language its own text is
        // in rather than to whichever was asserted first.
        let asserted = ctx.asserted_languages();
        if asserted.len() == 1 {
            return Ok(());
        }
        // A nonempty asserted set none of whose tags lingua supports yields no
        // detector: skip detection rather than widen to all languages.
        let Some(detector) = self.detector_for(&asserted) else {
            return Ok(());
        };
        // No recognizable text at this chunk (an un-transcribed clip, an
        // un-OCR'd image): no language to detect.
        let Some(text) = M::as_text(subject.data(), subject.artifact()) else {
            return Ok(());
        };
        // Detect into an owned list first so the immutable borrow of the payload
        // text ends before `detect_language` takes `&mut subject`.
        let detections = detector.detect(text)?;
        for detection in detections {
            subject.detect_language(detection);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use elide_core::modality::tabular::Tabular;
    use elide_core::modality::text::{Text, TextData};
    use elide_core::recognition::{Scope, Subject};

    use super::*;

    #[tokio::test]
    async fn detects_english_onto_input() {
        let data = TextData::new("The quick brown fox jumps over the lazy dog.");
        let scope = Scope::new();
        let ctx = Context::<Text>::new(&scope);
        let mut subject = Subject::new(data);
        LinguaEnricher::unrestricted()
            .enrich(&mut subject, &ctx)
            .await
            .unwrap();
        assert_eq!(
            ctx.languages(&subject).primary().unwrap().primary_subtag(),
            "en"
        );
    }

    #[tokio::test]
    async fn detects_onto_a_tabular_payload() {
        // Tabular cells are text, so the same enricher scopes a CSV/XLSX
        // pipeline to its detected language.
        let data = TextData::new("The quick brown fox jumps over the lazy dog.");
        let scope = Scope::new();
        let ctx = Context::<Tabular>::new(&scope);
        let mut subject = Subject::new(data);
        LinguaEnricher::unrestricted()
            .enrich(&mut subject, &ctx)
            .await
            .unwrap();
        assert_eq!(
            ctx.languages(&subject).primary().unwrap().primary_subtag(),
            "en"
        );
    }

    #[tokio::test]
    async fn single_asserted_language_skips_detection() {
        let de: LanguageTag = "de".parse().unwrap();
        let data = TextData::new("The quick brown fox");
        let scope = Scope::new().with_language(de);
        let ctx = Context::<Text>::new(&scope);
        let mut subject = Subject::new(data);
        LinguaEnricher::unrestricted()
            .enrich(&mut subject, &ctx)
            .await
            .unwrap();
        // Only the asserted German remains; English was never detected.
        assert!(subject.detected_languages().is_empty());
        assert_eq!(ctx.languages(&subject).ranked().len(), 1);
        assert_eq!(
            ctx.languages(&subject).primary().unwrap().primary_subtag(),
            "de"
        );
    }

    #[tokio::test]
    async fn multiple_asserted_languages_detect_within_the_asserted_set() {
        // Two asserted languages means a mixed document: detection still runs, so
        // each region can be attributed, but restricted to the asserted set. The
        // text is English, and English is one of the two asserted, so a spanned
        // `en` detection is added alongside the asserted claims.
        let scope = Scope::new()
            .with_language("pl".parse::<LanguageTag>().unwrap())
            .with_language("en".parse::<LanguageTag>().unwrap());
        let data = TextData::new("The quick brown fox jumps over the lazy dog.");
        let ctx = Context::<Text>::new(&scope);
        let mut subject = Subject::new(data);
        LinguaEnricher::unrestricted()
            .enrich(&mut subject, &ctx)
            .await
            .unwrap();
        let detected = subject.detected_languages();
        assert!(!detected.is_empty(), "detection runs for multiple asserted");
        // The fixture is English, so detection must actually attribute it to
        // English — not merely "one of the asserted set" (which would also pass
        // if the English text were mislabeled Polish, the exact failure this
        // change fixes).
        assert!(
            detected.iter().all(|c| c.language.primary_subtag() == "en"),
            "English text is attributed to English, got {:?}",
            detected
                .iter()
                .map(|c| c.language.primary_subtag().to_owned())
                .collect::<Vec<_>>(),
        );
        assert!(
            detected.iter().all(|c| c.span.is_some()),
            "detections carry spans for per-region attribution",
        );
    }

    #[tokio::test]
    async fn multiple_asserted_unsupported_languages_skip_detection() {
        // When two languages are asserted but none is one lingua supports, the
        // detector must not widen to all languages (which would stamp an
        // unrelated language); detection is skipped and no claim is added.
        let scope = Scope::new()
            .with_language("zz".parse::<LanguageTag>().unwrap())
            .with_language("qq".parse::<LanguageTag>().unwrap());
        let data = TextData::new("The quick brown fox jumps over the lazy dog.");
        let ctx = Context::<Text>::new(&scope);
        let mut subject = Subject::new(data);
        LinguaEnricher::unrestricted()
            .enrich(&mut subject, &ctx)
            .await
            .unwrap();
        assert!(
            subject.detected_languages().is_empty(),
            "no detection widens past an unsupported asserted set",
        );
    }
}
