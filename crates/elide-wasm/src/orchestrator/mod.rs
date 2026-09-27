//! The composed pipeline: a fluent [`Orchestrator`] that folds per-modality
//! detect-and-redact stages, and drives them over a blob with
//! [`redact`](Orchestrator::redact).

mod image;
mod report;

use elide::prelude::*;
use elide::{Orchestrator as CoreOrchestrator, Report as CoreReport};
use tsify::Ts;
use wasm_bindgen::prelude::*;

pub use self::report::{Entity, Location, Modality, Report};
use crate::analyzer::{Analyzer, StageModality};
use crate::anonymizer::Anonymizer;
use crate::error::{ElideError, ElideErrorKind};

/// The detect-and-redact pipeline, mirroring the toolkit's own
/// [`Orchestrator`](elide::Orchestrator): its configured modality stages plus the
/// built-in codec registry and label catalog.
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
                .with_scope(Scope::new().with_catalog(LabelCatalog::with_builtins())),
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

    /// Add a modality's stage: an [`Analyzer`] paired with an optional
    /// [`Anonymizer`] redaction policy (a default policy applies when omitted).
    /// The analyzer's modality selects the codec path. Consumes both handles.
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
                return Err(ElideError::new(
                    ElideErrorKind::Configuration,
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
                    .with_modality::<Metadata>(
                        image::metadata_analyzer(),
                        crate::anonymizer::default_image_metadata(),
                    )
                    .with_modality::<Image>(analyzer.build_image()?, pixels)
            }
        };
        Ok(self)
    }

    /// Detect and redact the personal data in `bytes`, whose format the `hint`
    /// names (a file extension like `txt`, `csv`, `png`), returning the redacted
    /// bytes and every finding.
    ///
    /// # Errors
    ///
    /// Rejects with an [`ElideError`] if the format is unknown or the pipeline
    /// fails.
    #[wasm_bindgen]
    pub async fn redact(&self, bytes: Vec<u8>, hint: String) -> Result<Ts<Report>, ElideError> {
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

/// The pipeline proper, returning the crate's own [`Result`] so the boundary
/// only deals with [`ElideError`].
async fn run(pipeline: &Orchestrator, bytes: Vec<u8>, hint: String) -> Result<Report> {
    let registry = FormatRegistry::with_builtin();
    let mut document = registry.document_with("input", &hint, bytes).await?;
    let report = pipeline
        .core()
        .anonymize(&mut document, &Directives::new())
        .await?;

    // Each modality locates its entities in its own coordinate space, so the
    // `Location` is built per modality (a text byte span, an image box, an audio
    // time span, a tabular cell, a metadata key).
    let mut entities = Vec::new();
    collect::<Text>(&report, &mut entities, Modality::Text, |e| {
        e.location
            .range()
            .map(|r| Location::Text {
                start: r.start,
                end: r.end,
            })
            .unwrap_or(Location::Text { start: 0, end: 0 })
    });
    collect::<Tabular>(&report, &mut entities, Modality::Tabular, |e| {
        Location::Tabular {
            sheet: e.location.sheet_name.as_ref().map(|s| s.to_string()),
            row: e.location.row_index,
            column: e.location.column_index,
        }
    });
    collect::<Image>(&report, &mut entities, Modality::Image, |e| {
        let bbox = &e.location.bounding_box;
        Location::Image {
            x: bbox.min.x,
            y: bbox.min.y,
            width: bbox.width(),
            height: bbox.height(),
        }
    });
    collect::<Audio>(&report, &mut entities, Modality::Audio, |e| {
        Location::Audio {
            start_ms: e.location.span.start_millis(),
            end_ms: e.location.span.end_millis(),
            speaker: e.location.speaker_id.as_ref().map(|s| s.to_string()),
        }
    });
    collect::<Metadata>(&report, &mut entities, Modality::Metadata, |e| {
        Location::Metadata {
            key: e.location.key.to_string(),
        }
    });

    let encoded = document.document.encode()?;
    Ok(Report {
        redacted: encoded.as_bytes().to_vec(),
        entities,
    })
}

/// Append modality `M`'s entities, mapping each one's location to a [`Location`]
/// with `locate`.
fn collect<M: elide::modality::Modality>(
    report: &CoreReport,
    out: &mut Vec<Entity>,
    modality: Modality,
    locate: impl Fn(&elide::entity::Entity<M>) -> Location,
) {
    let Some(entities) = report.entities::<M>() else {
        return;
    };
    for entity in entities {
        out.push(Entity {
            id: entity.id.to_string(),
            modality,
            label: entity.label.as_str().to_owned(),
            location: locate(entity),
            confidence: f32::from(entity.confidence),
            language: entity.language.as_ref().map(|l| l.to_string()),
            coref: entity.coref.as_ref().map(|c| c.as_str().to_owned()),
        });
    }
}
