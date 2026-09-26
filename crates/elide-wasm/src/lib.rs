#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

//! WebAssembly bindings that run the Elide detect-and-redact pipeline in a
//! browser, across every modality the browser can hand over as bytes.
//!
//! The pipeline is exposed to JavaScript as a set of opaque handles composed by
//! factory functions, mirroring the [`elide`] facade's own split between
//! detection and redaction. JavaScript builds recognizers (a
//! [`RecognizerHandle`](recognizer::RecognizerHandle) from patterns or a NER
//! callback), folds them into a modality on a [`PipelineBuilder`](pipeline::PipelineBuilder),
//! and drives the resulting [`PipelineHandle`](pipeline::PipelineHandle) over a
//! blob with [`redact`](pipeline::redact): the format hint picks the codec, the
//! codec decodes the bytes into a modality, and the pipeline dispatches to the
//! matching detect-and-redact stage. The rich Rust objects stay in wasm memory
//! behind the handles; only config and the
//! [`RedactionResult`](result::RedactionResult) cross the boundary as data.
//!
//! The whole pipeline is `async`, and on wasm its futures are driven by the
//! browser's own event loop through [`wasm_bindgen_futures`] — there is no Tokio
//! runtime.

pub mod enricher;
pub mod pipeline;
pub mod recognizer;
pub mod result;

use wasm_bindgen::prelude::*;

/// Install the panic hook once, so a Rust panic surfaces in the browser console
/// with a readable message instead of an opaque `unreachable`.
#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
}
