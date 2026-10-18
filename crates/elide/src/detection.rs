//! Detection: the [`Analyzer`] "find" engine and its reconciliation layers.
//!
//! The [`Analyzer`] runs the enrichers and recognizers, then reconciles
//! their findings through the [`Layer`] stages, [`calibrate`],
//! [`reconcile`], [`filter`], each reshaping or pruning the working entity
//! set. The [`escalation`] module composes recognizers — a cheap one that always
//! runs and a costly one a decider escalates to only when warranted.
//! Re-exported from [`elide_detection`].
//!
//! [`Analyzer`]: crate::detection::Analyzer
//! [`Layer`]: elide_detection::Layer
//! [`calibrate`]: elide_detection::calibrate
//! [`reconcile`]: elide_detection::reconcile
//! [`filter`]: elide_detection::filter

#[doc(inline)]
pub use elide_detection::{Analyzer, Layer, LayerOutput, calibrate, escalation, filter, reconcile};
