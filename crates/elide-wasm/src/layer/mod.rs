//! The detection layers a caller can configure on an [`Analyzer`](crate::analyzer::Analyzer).
//!
//! An analyzer runs its recognizers, then reshapes the findings through a stack
//! of [`Layer`](elide::detection::Layer)s — reconcile (merge or arbitrate
//! overlaps), then filter (drop low-confidence findings). This module exposes
//! those layers as opaque [`Layer`]s built by [`Layer::filter`],
//! [`Layer::reconcile_same_label`], and [`Layer::reconcile_cross_label`].
//!
//! A [`Layer`](elide::detection::Layer) is modality-generic — one layer works on
//! any stage — so a [`Layer`] handle is unbranded, unlike the enricher and
//! recognizer handles.

use elide::detection::Analyzer;
use elide::detection::filter::FilterLayer;
use elide::detection::reconcile::{Exclusive, Merging, Permissive, ReconcileLayer, Structural};
use elide::modality::Modality;
use elide::primitive::ConfidenceThreshold;
use serde::Deserialize;
use tsify::{Ts, Tsify};
use wasm_bindgen::prelude::*;

use crate::error::{ElideError, ElideErrorKind};

/// The scoring a same-label merge combines the pair's confidences with.
#[derive(Deserialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub enum SameLabelScoring {
    /// The most confident finding wins (the default).
    Max,
    /// Agreeing detectors accumulate evidence (`1 − ∏(1 − pᵢ)`).
    NoisyOr,
}

/// The tiebreaker an exclusive cross-label reconciler keeps a finding by.
#[derive(Deserialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub enum ExclusiveTiebreaker {
    /// Keep the higher-confidence finding.
    HighestConfidence,
    /// Keep the finding covering the longer span.
    LongestSpan,
}

/// How a cross-label reconciler resolves overlapping findings of different
/// labels.
#[derive(Deserialize, Tsify)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum CrossLabelConfig {
    /// Structural arbitration: overlaps at or above an IoU `iou` (default `0.5`)
    /// are a conflict, ties to the higher confidence.
    Structural {
        /// The IoU at or above which a non-nested overlap is a conflict.
        iou: Option<f32>,
    },
    /// Keep exactly one of any overlapping pair, by `tiebreaker`.
    Exclusive {
        /// Which finding of the pair to keep.
        tiebreaker: ExclusiveTiebreaker,
    },
    /// Keep every overlap; with `contesting`, flag each pair for review.
    Permissive {
        /// Flag each kept overlap as contested for a human edit step.
        contesting: Option<bool>,
    },
}

/// A same-label reconcile layer's scoring, in the shape [`LayerKind`] stores.
enum SameLabelKind {
    Max,
    NoisyOr,
}

/// A cross-label reconcile layer's configuration, in the shape [`LayerKind`]
/// stores.
enum CrossLabelKind {
    Structural { iou: Option<f32> },
    Exclusive(ExclusiveTiebreaker),
    Permissive { contesting: bool },
}

/// The layers a handle may carry, one variant per configurable stage.
enum LayerKind {
    /// Drop findings below a confidence threshold.
    Filter(ConfidenceThreshold),
    /// Merge overlapping same-label findings.
    SameLabel(SameLabelKind),
    /// Arbitrate overlapping different-label findings.
    CrossLabel(CrossLabelKind),
}

/// A detection layer, ready to fold into an [`Analyzer`](crate::analyzer::Analyzer).
///
/// Built with [`Layer::filter`], [`Layer::reconcile_same_label`], or
/// [`Layer::reconcile_cross_label`], and consumed by
/// [`Analyzer::layer`](crate::analyzer::Analyzer::layer). One class wraps every
/// layer kind; a layer applies to any modality, so the handle is unbranded.
#[wasm_bindgen]
pub struct Layer(LayerKind);

#[wasm_bindgen]
impl Layer {
    /// Build a filter layer that drops findings below `threshold` (in `[0, 1]`).
    ///
    /// # Errors
    ///
    /// Throws an [`ElideError`] if `threshold` is outside `[0, 1]`.
    #[wasm_bindgen(js_name = filter)]
    pub fn filter(threshold: f32) -> Result<Layer, ElideError> {
        let threshold = ConfidenceThreshold::new(threshold).ok_or_else(|| {
            ElideError::new(
                ElideErrorKind::Configuration,
                format!("filter threshold {threshold} is outside [0, 1]"),
            )
        })?;
        Ok(Self(LayerKind::Filter(threshold)))
    }

    /// Build a same-label reconcile layer that merges overlapping findings of the
    /// same label with the given `scoring`.
    ///
    /// # Errors
    ///
    /// Throws an [`ElideError`] if `scoring` is not a known scoring.
    #[wasm_bindgen(js_name = reconcileSameLabel)]
    pub fn reconcile_same_label(scoring: Ts<SameLabelScoring>) -> Result<Layer, ElideError> {
        let scoring = scoring.to_rust().map_err(|e| {
            ElideError::new(
                ElideErrorKind::Configuration,
                format!("invalid same-label scoring: {e}"),
            )
        })?;
        let kind = match scoring {
            SameLabelScoring::Max => SameLabelKind::Max,
            SameLabelScoring::NoisyOr => SameLabelKind::NoisyOr,
        };
        Ok(Self(LayerKind::SameLabel(kind)))
    }

    /// Build a cross-label reconcile layer that arbitrates overlapping findings
    /// of different labels per `config`.
    ///
    /// # Errors
    ///
    /// Throws an [`ElideError`] if `config` is not a known configuration.
    #[wasm_bindgen(js_name = reconcileCrossLabel)]
    pub fn reconcile_cross_label(config: Ts<CrossLabelConfig>) -> Result<Layer, ElideError> {
        let config = config.to_rust().map_err(|e| {
            ElideError::new(
                ElideErrorKind::Configuration,
                format!("invalid cross-label config: {e}"),
            )
        })?;
        let kind = match config {
            CrossLabelConfig::Structural { iou } => CrossLabelKind::Structural { iou },
            CrossLabelConfig::Exclusive { tiebreaker } => CrossLabelKind::Exclusive(tiebreaker),
            CrossLabelConfig::Permissive { contesting } => CrossLabelKind::Permissive {
                contesting: contesting.unwrap_or(false),
            },
        };
        Ok(Self(LayerKind::CrossLabel(kind)))
    }
}

impl Layer {
    /// Fold this layer into `analyzer` for its modality `M`.
    ///
    /// The analyzer stores each layer by concrete type (there is no boxed
    /// `Layer` it accepts), so this matches the kind and adds the built layer
    /// directly. Every concrete layer implements
    /// [`Layer`](elide::detection::Layer) for all modalities, so one handle
    /// serves any stage.
    pub(crate) fn apply<M: Modality>(self, analyzer: Analyzer<M>) -> Analyzer<M> {
        match self.0 {
            LayerKind::Filter(threshold) => {
                analyzer.with_layer(FilterLayer::new().with_threshold(threshold))
            }
            LayerKind::SameLabel(SameLabelKind::Max) => {
                analyzer.with_layer(ReconcileLayer::same_label(Merging::max()))
            }
            LayerKind::SameLabel(SameLabelKind::NoisyOr) => {
                analyzer.with_layer(ReconcileLayer::same_label(Merging::noisy_or()))
            }
            LayerKind::CrossLabel(CrossLabelKind::Structural { iou }) => match iou {
                Some(iou) => analyzer.with_layer(ReconcileLayer::cross_label(
                    Structural::standard().with_threshold(iou),
                )),
                None => analyzer.with_layer(ReconcileLayer::cross_label(Structural::default())),
            },
            LayerKind::CrossLabel(CrossLabelKind::Exclusive(
                ExclusiveTiebreaker::HighestConfidence,
            )) => analyzer.with_layer(ReconcileLayer::cross_label(Exclusive::highest_confidence())),
            LayerKind::CrossLabel(CrossLabelKind::Exclusive(ExclusiveTiebreaker::LongestSpan)) => {
                analyzer.with_layer(ReconcileLayer::cross_label(Exclusive::longest_span()))
            }
            LayerKind::CrossLabel(CrossLabelKind::Permissive { contesting: false }) => {
                analyzer.with_layer(ReconcileLayer::cross_label(Permissive::new()))
            }
            LayerKind::CrossLabel(CrossLabelKind::Permissive { contesting: true }) => {
                analyzer.with_layer(ReconcileLayer::cross_label(Permissive::contesting()))
            }
        }
    }
}
