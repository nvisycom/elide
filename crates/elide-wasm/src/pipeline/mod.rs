//! The composed pipeline: a fluent [`PipelineBuilder`] that folds per-modality
//! detect-and-redact stages into one [`PipelineHandle`] (an [`Orchestrator`]),
//! and the [`redact`] entry point that drives it over a blob.

mod audio;
mod image;
mod tabular;
mod text;

use elide::prelude::*;
use tsify::Ts;
use wasm_bindgen::prelude::*;

use crate::enricher::{AudioEnricherHandle, ImageEnricherHandle, TextEnricherHandle};
use crate::recognizer::RecognizerHandle;

/// A fluent builder for a [`PipelineHandle`].
///
/// Add the modalities the app handles — [`with_text`](Self::with_text) folds the
/// caller's recognizers into a text stage — then [`build`](Self::build) freezes
/// it into a handle. Each `with_*` consumes the handles it is given.
#[wasm_bindgen]
#[derive(Default)]
pub struct PipelineBuilder {
    orchestrator: Orchestrator,
}

#[wasm_bindgen]
impl PipelineBuilder {
    /// A new, empty builder.
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add the text modality: detect with the given `recognizers`, redact with
    /// the default text policy. An optional `language` enricher resolves the
    /// input's language first. Consumes each handle.
    #[wasm_bindgen(js_name = withText)]
    pub fn with_text(
        mut self,
        recognizers: Vec<RecognizerHandle>,
        language: Option<TextEnricherHandle>,
    ) -> Self {
        self.orchestrator = self.orchestrator.with_modality::<Text>(
            text::analyzer(recognizers, language.map(TextEnricherHandle::into_enricher)),
            text::anonymizer(),
        );
        self
    }

    /// Add the tabular modality: the same text `recognizers` scan each cell,
    /// redacted with the default cell policy. An optional `language` enricher
    /// resolves the language first. Consumes each handle.
    #[wasm_bindgen(js_name = withTabular)]
    pub fn with_tabular(
        mut self,
        recognizers: Vec<RecognizerHandle>,
        language: Option<TextEnricherHandle>,
    ) -> Self {
        self.orchestrator = self.orchestrator.with_modality::<Tabular>(
            tabular::analyzer(recognizers, language.map(TextEnricherHandle::into_enricher)),
            tabular::anonymizer(),
        );
        self
    }

    /// Add the image modality: always scrub the image's privacy-relevant EXIF
    /// metadata, and — when an `ocr` enricher is given — read the image's text
    /// with it, scan it with `recognizers`, and black out matched regions.
    /// Consumes each handle.
    #[wasm_bindgen(js_name = withImage)]
    pub fn with_image(
        mut self,
        ocr: Option<ImageEnricherHandle>,
        recognizers: Vec<RecognizerHandle>,
    ) -> Self {
        self.orchestrator = self
            .orchestrator
            .with_modality::<Metadata>(image::metadata_analyzer(), image::metadata_anonymizer());
        if let Some(ocr) = ocr {
            self.orchestrator = self.orchestrator.with_modality::<Image>(
                image::pixel_analyzer(ocr, recognizers),
                image::pixel_anonymizer(),
            );
        }
        self
    }

    /// Add the audio modality: the `stt` enricher transcribes the clip, the
    /// `recognizers` scan the transcript, and matched spans are silenced. With
    /// no STT enricher, audio round-trips unchanged. Consumes each handle.
    #[wasm_bindgen(js_name = withAudio)]
    pub fn with_audio(
        mut self,
        stt: Option<AudioEnricherHandle>,
        recognizers: Vec<RecognizerHandle>,
    ) -> Self {
        self.orchestrator = self.orchestrator.with_modality::<Audio>(
            audio::analyzer(recognizers, stt.map(AudioEnricherHandle::into_enricher)),
            audio::anonymizer(),
        );
        self
    }

    /// Freeze the builder into a runnable [`PipelineHandle`].
    #[wasm_bindgen]
    pub fn build(self) -> PipelineHandle {
        PipelineHandle {
            orchestrator: self
                .orchestrator
                .with_registry(FormatRegistry::with_builtin())
                .with_scope(Scope::new().with_catalog(LabelCatalog::with_builtins())),
        }
    }
}

/// A built pipeline: an [`Orchestrator`] with the configured modality stages,
/// the built-in codec registry, and the built-in label catalog. Borrowed by
/// [`redact`] and reusable across calls.
#[wasm_bindgen]
pub struct PipelineHandle {
    orchestrator: Orchestrator,
}

impl PipelineHandle {
    /// The wrapped orchestrator, for the redact entry point.
    pub(crate) fn orchestrator(&self) -> &Orchestrator {
        &self.orchestrator
    }
}

/// Detect and redact the personal data in `bytes`, whose format the `hint`
/// names (a file extension like `txt`, `csv`, `png`), returning the redacted
/// bytes and every finding.
///
/// # Errors
///
/// Rejects with a JS error if the format is unknown or the pipeline fails.
#[wasm_bindgen]
pub async fn redact(
    pipeline: &PipelineHandle,
    bytes: Vec<u8>,
    hint: String,
) -> Result<Ts<crate::result::RedactionResult>, JsError> {
    let result = run(pipeline, bytes, hint)
        .await
        .map_err(|e| JsError::new(&e.to_string()))?;
    Ts::from_rust(&result).map_err(|e| JsError::new(&e.to_string()))
}

/// The pipeline proper, returning the crate's own [`Result`] so the boundary
/// only deals with `JsError`.
async fn run(
    pipeline: &PipelineHandle,
    bytes: Vec<u8>,
    hint: String,
) -> Result<crate::result::RedactionResult> {
    use crate::result::{ByteRange, Finding, RedactionResult};

    let registry = FormatRegistry::with_builtin();
    let mut document = registry.document_with("input", &hint, bytes).await?;
    let report = pipeline
        .orchestrator()
        .anonymize(&mut document, &Directives::new())
        .await?;

    let mut findings = Vec::new();

    // Text is the one modality whose location is a byte range in the decoded
    // stream; the rest are located in their own coordinate space (a cell, a time
    // span, a metadata key), so they carry no `range`.
    if let Some(entities) = report.entities::<Text>() {
        for entity in entities {
            findings.push(Finding {
                modality: Text::NAME.to_owned(),
                label: entity.label.as_str().to_owned(),
                range: entity.location.range().map(|r| ByteRange {
                    start: r.start,
                    end: r.end,
                }),
                confidence: f32::from(entity.confidence),
            });
        }
    }
    collect_rangeless::<Tabular>(&report, &mut findings);
    collect_rangeless::<Metadata>(&report, &mut findings);
    collect_rangeless::<Image>(&report, &mut findings);
    collect_rangeless::<Audio>(&report, &mut findings);

    let encoded = document.document.encode()?;
    Ok(RedactionResult {
        redacted: encoded.as_bytes().to_vec(),
        findings,
    })
}

/// Append the findings of a non-text modality `M`, whose location is not a byte
/// range, so each finding carries only its label and confidence.
fn collect_rangeless<M: Modality>(report: &Report, findings: &mut Vec<crate::result::Finding>) {
    if let Some(entities) = report.entities::<M>() {
        for entity in entities {
            findings.push(crate::result::Finding {
                modality: M::NAME.to_owned(),
                label: entity.label.as_str().to_owned(),
                range: None,
                confidence: f32::from(entity.confidence),
            });
        }
    }
}
