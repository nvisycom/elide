//! [`LinguaEnricher`]: a language-detection [`Enricher`] backed by the
//! [`lingua`] crate.
//!
//! [`lingua`]: https://crates.io/crates/lingua

use std::sync::Arc;

use elide_core::enrichment::Enricher;
use elide_core::modality::TextRecognizable;
use elide_core::primitive::LanguageTag;
use elide_core::recognition::{Context, Subject};
use elide_core::{Component, Error, Result};

use crate::lingua_detector::{LinguaDetector, compiled_language};

/// The `tracing` target of this crate's events.
const TARGET: &str = "elide_lingua";

/// An enricher that detects the languages of text with the `lingua` crate,
/// and adds them to the subject for the stages that follow.
///
/// Its candidate languages are fixed at construction, and its detector is
/// built once and shared by its clones. A caller that asserts languages
/// overrides them for the call:
///
/// - one asserted language is authoritative, so detection is skipped;
/// - two or more mean a mixed document, so detection runs over just the
///   asserted languages, attributing each region to the one it is in; when
///   `lingua` was compiled with none of them, detection is skipped rather
///   than widened to languages the caller never declared.
#[derive(Debug, Clone, Component)]
pub struct LinguaEnricher {
    detector: Arc<LinguaDetector>,
}

impl LinguaEnricher {
    /// An enricher over every language `lingua` was compiled with.
    #[must_use]
    pub fn unrestricted() -> Self {
        Self {
            detector: Arc::new(LinguaDetector::for_all_languages()),
        }
    }

    /// An enricher over `candidates`: fewer candidates detect faster and more
    /// precisely. Candidates `lingua` was not compiled with are skipped, with
    /// a warning.
    ///
    /// # Errors
    ///
    /// [`Configuration`](elide_core::ErrorKind::Configuration) when `lingua`
    /// was compiled with none of `candidates`; its `common` and `all` features
    /// add languages.
    pub fn restricted(candidates: impl IntoIterator<Item = LanguageTag>) -> Result<Self> {
        let candidates: Vec<LanguageTag> = candidates.into_iter().collect();
        let detector = LinguaDetector::for_languages(&candidates).ok_or_else(|| {
            let tags: Vec<&str> = candidates.iter().map(LanguageTag::as_str).collect();
            Error::configuration(format!(
                "lingua was compiled with none of the candidate languages [{}]",
                tags.join(", ")
            ))
        })?;
        let skipped: Vec<&str> = candidates
            .iter()
            .filter(|tag| compiled_language(tag).is_none())
            .map(LanguageTag::as_str)
            .collect();
        if !skipped.is_empty() {
            tracing::warn!(
                target: TARGET,
                skipped = ?skipped,
                "lingua was not compiled with these candidate languages; enable its `common` or `all` feature to detect them",
            );
        }
        Ok(Self {
            detector: Arc::new(detector),
        })
    }
}

impl Default for LinguaEnricher {
    fn default() -> Self {
        Self::unrestricted()
    }
}

/// Detects language over any [`TextRecognizable`] modality: `Text` and
/// `Tabular` project their payload as text identically, and an audio transcript
/// or OCR layout is read from the subject's artifact, so the same enricher
/// scopes a text, tabular, transcript, or OCR pipeline to its language.
#[async_trait::async_trait]
impl<M: TextRecognizable> Enricher<M> for LinguaEnricher {
    async fn enrich(&self, subject: &mut Subject<M>, ctx: &Context<'_>) -> Result<()> {
        let asserted = ctx.scope().languages();
        let call_detector;
        let detector = match asserted {
            [] => &*self.detector,
            [claim] => {
                tracing::debug!(
                    target: TARGET,
                    language = claim.language.as_str(),
                    "one language asserted; detection skipped",
                );
                return Ok(());
            }
            claims => {
                let Some(detector) =
                    LinguaDetector::for_languages(claims.iter().map(|claim| &claim.language))
                else {
                    tracing::debug!(
                        target: TARGET,
                        asserted = claims.len(),
                        "lingua was compiled with none of the asserted languages; detection skipped",
                    );
                    return Ok(());
                };
                call_detector = detector;
                &call_detector
            }
        };
        // No recognizable text at this chunk (an un-transcribed clip, an
        // un-OCR'd image): no language to detect.
        let Some(text) = M::as_text(subject.data(), subject.artifact()) else {
            return Ok(());
        };
        let claims = detector.detect(text);
        tracing::debug!(
            target: TARGET,
            languages = ?claims.iter().map(|claim| claim.language.as_str()).collect::<Vec<_>>(),
            "languages detected",
        );
        for claim in claims {
            subject.detect_language(claim);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use elide_core::ErrorKind;
    use elide_core::modality::tabular::Tabular;
    use elide_core::modality::text::{Text, TextData};
    use elide_core::recognition::{Scope, Subject};

    use super::*;

    #[tokio::test]
    async fn detects_english_onto_input() {
        let data = TextData::new("The quick brown fox jumps over the lazy dog.");
        let scope = Scope::new();
        let ctx = Context::new(&scope);
        let mut subject = Subject::<Text>::new(data);
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
        let ctx = Context::new(&scope);
        let mut subject = Subject::<Tabular>::new(data);
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
        let ctx = Context::new(&scope);
        let mut subject = Subject::<Text>::new(data);
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
        let ctx = Context::new(&scope);
        let mut subject = Subject::<Text>::new(data);
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
        let ctx = Context::new(&scope);
        let mut subject = Subject::<Text>::new(data);
        LinguaEnricher::unrestricted()
            .enrich(&mut subject, &ctx)
            .await
            .unwrap();
        assert!(
            subject.detected_languages().is_empty(),
            "no detection widens past an unsupported asserted set",
        );
    }

    #[tokio::test]
    async fn a_restricted_enricher_detects_within_its_candidates() {
        let enricher = LinguaEnricher::restricted(["en".parse().unwrap()]).unwrap();
        let scope = Scope::new();
        let ctx = Context::new(&scope);
        let mut subject = Subject::<Text>::new(TextData::new(
            "The quick brown fox jumps over the lazy dog.",
        ));
        enricher.enrich(&mut subject, &ctx).await.unwrap();
        assert_eq!(
            ctx.languages(&subject).primary().unwrap().primary_subtag(),
            "en"
        );
    }

    #[test]
    fn restricting_to_no_compiled_language_is_a_configuration_error() {
        let error = LinguaEnricher::restricted(["zz".parse().unwrap()]).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Configuration);
        assert!(LinguaEnricher::restricted([]).is_err());
    }
}
