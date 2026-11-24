//! The composed pipeline: an [`Orchestrator`] of detect-and-redact stages.
//!
//! The fluent [`Orchestrator`] folds per-modality stages and drives them over a
//! blob with [`redact`](Orchestrator::redact).

mod entities;
mod report;

use elide::Orchestrator as CoreOrchestrator;
use elide::prelude::*;
use tsify::Ts;
use wasm_bindgen::prelude::*;

pub use self::report::{
    Entity, HiddenContent, HiddenContentKind, LeakProfile, Location, Modality, Rect, Redaction,
    Rendered, Report, Source, Span, Stripped,
};
use crate::analyzer::{Analyzer, StageModality};
use crate::anonymizer::Anonymizer;
use crate::error::{ElideError, ElideErrorKind, language_tags};

/// The detect-and-redact pipeline: modality stages, codecs, and labels.
///
/// It mirrors the toolkit's own [`Orchestrator`](elide::Orchestrator): its
/// configured modality stages plus the built-in codec registry and label
/// catalog.
///
/// Add a modality's stage with [`with`](Self::with) — an [`Analyzer`] paired with
/// the modality's redaction policy — then run it over a blob with
/// [`redact`](Self::redact). Reusable across calls.
#[wasm_bindgen]
pub struct Orchestrator {
    orchestrator: CoreOrchestrator,
}

impl Default for Orchestrator {
    fn default() -> Self {
        Self {
            orchestrator: CoreOrchestrator::new()
                .with_registry(FormatRegistry::with_builtin())
                .with_scope(scope()),
        }
    }
}

#[wasm_bindgen]
impl Orchestrator {
    /// A new orchestrator, seeded with the built-in codec registry and label
    /// catalog and no modality stages yet.
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a modality's stage: an [`Analyzer`] and an optional [`Anonymizer`].
    ///
    /// The [`Anonymizer`] is the redaction policy; a default one applies when
    /// omitted. The analyzer's modality selects the codec path. Consumes both
    /// handles.
    ///
    /// Text, tabular, and audio each detect-and-redact their own decoded content;
    /// image always scrubs the picture's EXIF metadata and, with an OCR-enriched
    /// analyzer, also redacts matched pixel regions.
    ///
    /// # Errors
    ///
    /// Throws an [`ElideError`] if an enricher does not apply to the analyzer's
    /// modality, if a rule's operator does not, or if `anonymizer`'s modality
    /// differs from the analyzer's.
    #[wasm_bindgen]
    pub fn with(
        mut self,
        analyzer: Analyzer,
        anonymizer: Option<Anonymizer>,
    ) -> Result<Orchestrator, ElideError> {
        let modality = analyzer.modality();
        let anonymizer = match anonymizer {
            Some(anonymizer) if anonymizer.modality() as u8 != modality as u8 => {
                return Err(ElideError::configuration(
                    "the anonymizer's modality does not match the analyzer's",
                ));
            }
            other => other,
        };
        self.orchestrator = match modality {
            StageModality::Text => {
                let anon = match anonymizer {
                    Some(a) => a.build_text()?,
                    None => crate::anonymizer::default_text(),
                };
                self.orchestrator
                    .with_modality::<Text>(analyzer.build_text()?, anon)
            }
            StageModality::Tabular => {
                let anon = match anonymizer {
                    Some(a) => a.build_tabular()?,
                    None => crate::anonymizer::default_text_tabular(),
                };
                self.orchestrator
                    .with_modality::<Tabular>(analyzer.build_tabular()?, anon)
            }
            StageModality::Audio => {
                let anon = match anonymizer {
                    Some(a) => a.build_audio()?,
                    None => crate::anonymizer::default_audio(),
                };
                self.orchestrator
                    .with_modality::<Audio>(analyzer.build_audio()?, anon)
            }
            StageModality::Image => {
                let pixels = match anonymizer {
                    Some(a) => a.build_image()?,
                    None => crate::anonymizer::default_image_pixels(),
                };
                self.orchestrator
                    .with_modality::<Image>(analyzer.build_image()?, pixels)
            }
        };
        Ok(self)
    }

    /// Assert the languages (BCP-47 tags such as `de`) every blob this
    /// orchestrator redacts is written in, mirroring the toolkit's
    /// `Scope::with_languages`.
    ///
    /// An asserted language outranks a detected one: recognizers scoped to a
    /// language run by it, and a backend's language routes follow it. Replaces
    /// any languages asserted before; an empty list asserts none, leaving the
    /// language to detection.
    ///
    /// # Errors
    ///
    /// Throws an [`ElideError`] if a tag is not valid BCP-47.
    #[wasm_bindgen(js_name = withLanguages)]
    pub fn with_languages(mut self, languages: Vec<String>) -> Result<Orchestrator, ElideError> {
        let languages = language_tags(&languages)?;
        self.orchestrator = self
            .orchestrator
            .with_scope(scope().with_languages(languages));
        Ok(self)
    }

    /// Detect and redact the personal data in `bytes`, of the format `hint`
    /// names.
    ///
    /// The hint is a file extension like `txt`, `docx`, `pdf`. Without one,
    /// the format is recognized by its leading bytes, which only the binary
    /// formats carry. Returns the redacted bytes and every finding.
    ///
    /// # Errors
    ///
    /// Rejects with an [`ElideError`] if the format is unknown or the pipeline
    /// fails.
    #[wasm_bindgen]
    pub async fn redact(
        &self,
        bytes: Vec<u8>,
        hint: Option<String>,
    ) -> Result<Ts<Report>, ElideError> {
        let result = run(self, bytes, hint).await?;
        Ts::from_rust(&result).map_err(|e| {
            ElideError::new(
                ElideErrorKind::Processing,
                format!("could not serialize the redaction result: {e}"),
            )
        })
    }
}

impl Orchestrator {
    /// The wrapped core orchestrator, for the redact entry point.
    pub(crate) fn core(&self) -> &CoreOrchestrator {
        &self.orchestrator
    }
}

/// The scope every run starts from: the built-in label catalog, and no asserted
/// language.
fn scope() -> Scope {
    Scope::new().with_catalog(LabelCatalog::with_builtins())
}

/// The pipeline proper, returning the crate's own [`Result`] so the boundary
/// only deals with [`ElideError`].
async fn run(pipeline: &Orchestrator, bytes: Vec<u8>, hint: Option<String>) -> Result<Report> {
    let registry = FormatRegistry::with_builtin();
    let policy = HiddenContentPolicy::default();
    let mut document = match hint.as_deref().filter(|hint| !hint.is_empty()) {
        Some(hint) => registry.document_with("input", hint, bytes, policy).await?,
        None => {
            let decoded = registry.decode_content(bytes.into(), policy).await?;
            Document::new("input", decoded)
        }
    };
    // Detect, read what each entity matched while the document still holds
    // it, then redact.
    let core = pipeline.core();
    let analyzed = core.analyze(&mut document).await?;
    let values = self::entities::read(core, &mut document, &analyzed).await;
    let report = core.anonymize_with(&mut document, analyzed.report).await?;
    let entities = self::entities::entities(&report, values);

    let stripped = report
        .stripped()
        .map(|item| Stripped {
            part: item.part.segments().map(str::to_owned).collect(),
            kind: item.content.kind().into(),
            item: item.content.into(),
            count: item.count,
        })
        .collect();

    let encoded = document.document.encode()?;
    Ok(Report {
        redacted: encoded.as_bytes().to_vec(),
        entities,
        stripped,
    })
}
