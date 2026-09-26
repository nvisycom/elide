#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

//! WebAssembly bindings that run the Elide detect-and-redact pipeline in a
//! browser, across every modality the browser can hand over as bytes.
//!
//! The pipeline is exposed to JavaScript as a set of opaque handles composed by
//! static factory methods, mirroring the [`elide`] facade's own split between
//! detection and redaction. JavaScript builds recognizers (a
//! [`Recognizer`](recognizer::Recognizer) from patterns or a NER callback) and
//! enrichers, folds them into an [`Analyzer`](analyzer::Analyzer) per modality,
//! adds each stage to an [`Orchestrator`](orchestrator::Orchestrator), and drives it
//! over a blob with [`redact`](orchestrator::Orchestrator::redact): the format hint picks the
//! codec, the codec decodes the bytes into a modality, and the pipeline
//! dispatches to the matching detect-and-redact stage. The rich Rust objects
//! stay in wasm memory behind the handles; only config and the
//! [`Report`](orchestrator::Report) cross the boundary as data.
//!
//! The whole pipeline is `async`, and on wasm its futures are driven by the
//! browser's own event loop through [`wasm_bindgen_futures`] — there is no Tokio
//! runtime.

pub mod analyzer;
pub mod anonymizer;
pub mod enricher;
pub mod error;
pub mod layer;
pub mod operator;
pub mod orchestrator;
pub mod recognizer;
pub mod rule;

use wasm_bindgen::prelude::*;

/// Install the panic hook once, so a Rust panic surfaces in the browser console
/// with a readable message instead of an opaque `unreachable`.
#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
}

/// Every built-in label id, in catalog order (`email_address`, `phone_number`,
/// …). The single source of truth for the generated `Label` constants in the
/// `@nvisy/elide` package; see the `labels` example that emits them as JSON.
pub fn builtin_label_ids() -> Vec<String> {
    elide::entity::LabelCatalog::with_builtins()
        .iter()
        .map(|label| label.id().to_owned())
        .collect()
}
