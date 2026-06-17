//! Wire types for the externalised `inference-gliner` Bento.
//!
//! Mirror of `nvisy_core.ner.v1` from [`nvisycom/inference`].
//! Schema version v1. Field names are camelCase on the wire to
//! match the Python service.
//!
//! [`BentoBackend`]: super::BentoBackend
//! [`nvisycom/inference`]: https://github.com/nvisycom/inference

use serde::{Deserialize, Serialize};

/// Outer batch wrapper. Single- and multi-text calls share the
/// same shape — a single recognition is just a batch of one.
#[derive(Serialize)]
pub(super) struct WireBatch {
    /// One recognition request per text. Response array has the
    /// same length and ordering.
    pub requests: Vec<WireRequest>,
}

/// One recognition request: a text plus the per-call inference
/// knobs the GLiNER model exposes.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct WireRequest {
    /// The text to recognise entities in.
    pub text: String,
    /// Entity label names the caller is interested in. GLiNER is
    /// zero-shot — sending an empty list is meaningless and the
    /// runtime short-circuits before making the call.
    pub labels: Vec<String>,
    /// Lower bound on per-entity score. The runtime keeps this at
    /// `0.0` and post-filters locally so threshold decisions stay
    /// in one place (the engine-side detection driver).
    pub threshold: f64,
    /// Optional BCP-47 language hint. Multilingual GLiNER variants
    /// may ignore it; monolingual variants may validate it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
}

/// One per-request recognition result.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct WireResponse {
    /// Identity of the underlying model that produced these
    /// entities — surfaced onto the entity's trail step for
    /// audit attribution. Echoed per response so a service hosting
    /// multiple models can report different identities within one
    /// batch.
    #[allow(dead_code)]
    pub model: String,
    /// Recognised entities. Each entity's label is the
    /// service-side classification, returned as a string so the
    /// recognizer's [`LabelMap`] can re-canonicalise it.
    ///
    /// [`LabelMap`]: nvisy_core::recognition::LabelMap
    #[serde(default)]
    pub entities: Vec<WireEntity>,
}

/// One recognised entity span on the wire.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct WireEntity {
    /// Service-side classification of this span. Translated to
    /// the workspace label vocabulary by the recognizer's
    /// `LabelMap`.
    pub label: String,
    /// Raw model score in `[0.0, 1.0]`.
    pub score: f64,
    /// Byte offset of the entity's start within
    /// [`WireRequest::text`].
    pub start: usize,
    /// Byte offset one past the entity's end within
    /// [`WireRequest::text`] (half-open `start..end`).
    pub end: usize,
}
