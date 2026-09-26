//! The [`Analyzer`]: the detect side of a modality's stage, mirroring the Rust
//! [`Analyzer`](elide::detection::Analyzer) builder.
//!
//! A caller builds one per modality — [`Analyzer::text`], [`Analyzer::image`], … —
//! then folds in enrichers, recognizers, and layers with [`enrich`],
//! [`recognize`], and [`layer`], and hands it to
//! [`Orchestrator::with`](crate::orchestrator::Orchestrator::with). The
//! modality is fixed at construction; an enricher that does not apply to it, or
//! (in TypeScript) a mismatched handle, is rejected.
//!
//! [`enrich`]: Analyzer::enrich
//! [`recognize`]: Analyzer::recognize
//! [`layer`]: Analyzer::layer

use elide::detection::Analyzer as RustAnalyzer;
use elide::detection::filter::FilterLayer;
use elide::detection::reconcile::{Merging, ReconcileLayer, Structural};
use elide::modality::Modality;
use elide::modality::audio::Audio;
use elide::modality::image::Image;
use elide::modality::tabular::Tabular;
use elide::modality::text::Text;
use elide::primitive::ConfidenceThreshold;
use wasm_bindgen::prelude::*;

use crate::enricher::Enricher;
use crate::error::ElideError;
use crate::layer::Layer;
use crate::recognizer::Recognizer;

/// Which modality an [`Analyzer`] stage detects over.
#[derive(Clone, Copy)]
pub(crate) enum StageModality {
    Text,
    Tabular,
    Image,
    Audio,
}

/// The detect side of a modality's stage: enrichers, recognizers, and layers,
/// fixed to one modality.
///
/// Mirrors the Rust [`Analyzer`](elide::detection::Analyzer) builder. Built with
/// [`Analyzer::text`] / [`Analyzer::tabular`] / [`Analyzer::image`] /
/// [`Analyzer::audio`] and consumed by
/// [`Orchestrator::with`](crate::orchestrator::Orchestrator::with).
#[wasm_bindgen]
pub struct Analyzer {
    modality: StageModality,
    enrichers: Vec<Enricher>,
    recognizers: Vec<Recognizer>,
    layers: Vec<Layer>,
}

#[wasm_bindgen]
impl Analyzer {
    /// An analyzer for the text modality.
    #[wasm_bindgen(js_name = text)]
    pub fn text() -> Analyzer {
        Analyzer::new(StageModality::Text)
    }

    /// An analyzer for the tabular modality (CSV cells).
    #[wasm_bindgen(js_name = tabular)]
    pub fn tabular() -> Analyzer {
        Analyzer::new(StageModality::Tabular)
    }

    /// An analyzer for the image modality (OCR-read pixel text).
    #[wasm_bindgen(js_name = image)]
    pub fn image() -> Analyzer {
        Analyzer::new(StageModality::Image)
    }

    /// An analyzer for the audio modality (an STT transcript).
    #[wasm_bindgen(js_name = audio)]
    pub fn audio() -> Analyzer {
        Analyzer::new(StageModality::Audio)
    }

    /// Add an enricher to run before recognition. Consumes the handle.
    #[wasm_bindgen(js_name = enrich)]
    pub fn enrich(mut self, enricher: Enricher) -> Analyzer {
        self.enrichers.push(enricher);
        self
    }

    /// Add a recognizer. Consumes the handle.
    #[wasm_bindgen(js_name = recognize)]
    pub fn recognize(mut self, recognizer: Recognizer) -> Analyzer {
        self.recognizers.push(recognizer);
        self
    }

    /// Add a reconcile or filter layer, in the order it should run. Consumes the
    /// handle. With no layers added, the stage uses the default reconcile-and-
    /// filter stack.
    #[wasm_bindgen(js_name = layer)]
    pub fn layer(mut self, layer: Layer) -> Analyzer {
        self.layers.push(layer);
        self
    }
}

impl Analyzer {
    fn new(modality: StageModality) -> Self {
        Self {
            modality,
            enrichers: Vec::new(),
            recognizers: Vec::new(),
            layers: Vec::new(),
        }
    }

    /// This analyzer's modality.
    pub(crate) fn modality(&self) -> StageModality {
        self.modality
    }

    /// Build the Rust [`Analyzer<Text>`](elide::detection::Analyzer).
    ///
    /// # Errors
    ///
    /// Errors if an enricher does not apply to text.
    pub(crate) fn build_text(self) -> Result<RustAnalyzer<Text>, ElideError> {
        let mut analyzer = RustAnalyzer::new();
        for enricher in self.enrichers {
            analyzer = enricher.apply_text(analyzer)?;
        }
        for recognizer in self.recognizers {
            analyzer = analyzer.with_recognizer(recognizer.into_recognizer::<Text>());
        }
        Ok(with_layers(analyzer, self.layers))
    }

    /// Build the Rust [`Analyzer<Tabular>`](elide::detection::Analyzer).
    ///
    /// # Errors
    ///
    /// Errors if an enricher does not apply to tabular.
    pub(crate) fn build_tabular(self) -> Result<RustAnalyzer<Tabular>, ElideError> {
        let mut analyzer = RustAnalyzer::new();
        for enricher in self.enrichers {
            analyzer = enricher.apply_tabular(analyzer)?;
        }
        for recognizer in self.recognizers {
            analyzer = analyzer.with_recognizer(recognizer.into_recognizer::<Tabular>());
        }
        Ok(with_layers(analyzer, self.layers))
    }

    /// Build the Rust [`Analyzer<Image>`](elide::detection::Analyzer).
    ///
    /// # Errors
    ///
    /// Errors if an enricher does not apply to image.
    pub(crate) fn build_image(self) -> Result<RustAnalyzer<Image>, ElideError> {
        let mut analyzer = RustAnalyzer::new();
        for enricher in self.enrichers {
            analyzer = enricher.apply_image(analyzer)?;
        }
        for recognizer in self.recognizers {
            analyzer = analyzer.with_recognizer(recognizer.into_recognizer::<Image>());
        }
        Ok(with_layers(analyzer, self.layers))
    }

    /// Build the Rust [`Analyzer<Audio>`](elide::detection::Analyzer).
    ///
    /// # Errors
    ///
    /// Errors if an enricher does not apply to audio.
    pub(crate) fn build_audio(self) -> Result<RustAnalyzer<Audio>, ElideError> {
        let mut analyzer = RustAnalyzer::new();
        for enricher in self.enrichers {
            analyzer = enricher.apply_audio(analyzer)?;
        }
        for recognizer in self.recognizers {
            analyzer = analyzer.with_recognizer(recognizer.into_recognizer::<Audio>());
        }
        Ok(with_layers(analyzer, self.layers))
    }
}

/// Fold the configured `layers` into `analyzer`, or the default reconcile-and-
/// filter stack when none were configured.
fn with_layers<M: Modality>(mut analyzer: RustAnalyzer<M>, layers: Vec<Layer>) -> RustAnalyzer<M> {
    if layers.is_empty() {
        return analyzer
            .with_layer(ReconcileLayer::same_label(Merging::max()))
            .with_layer(ReconcileLayer::cross_label(Structural::default()))
            .with_layer(FilterLayer::new().with_threshold(ConfidenceThreshold::BASELINE));
    }
    for layer in layers {
        analyzer = layer.apply(analyzer);
    }
    analyzer
}
