//! The detection side of the pipeline: recognizers folded into an analyzer.

mod pattern;

use elide::prelude::*;
use wasm_bindgen::prelude::*;

pub use self::pattern::{RecognizerHandle, create_pattern_recognizer};

/// A built analyzer: its recognizers plus the demo's reconcile-and-filter
/// pipeline. Borrowed by [`redact`](crate::redact) and reusable across calls.
#[wasm_bindgen]
pub struct AnalyzerHandle(Analyzer<Text>);

impl AnalyzerHandle {
    /// The wrapped analyzer, for the redact entry point.
    pub(crate) fn analyzer(&self) -> &Analyzer<Text> {
        &self.0
    }
}

/// Fold one or more recognizers into an analyzer with the demo's reconcile and
/// filter layers. Consumes each recognizer handle.
#[wasm_bindgen(js_name = createAnalyzer)]
pub fn create_analyzer(recognizers: Vec<RecognizerHandle>) -> AnalyzerHandle {
    let mut analyzer = Analyzer::new();
    for recognizer in recognizers {
        analyzer = analyzer.with_recognizer(recognizer.into_inner());
    }
    let analyzer = analyzer
        .with_layer(ReconcileLayer::same_label(Merging::max()))
        .with_layer(ReconcileLayer::cross_label(Structural::default()))
        .with_layer(FilterLayer::new().with_threshold(ConfidenceThreshold::BASELINE));
    AnalyzerHandle(analyzer)
}
