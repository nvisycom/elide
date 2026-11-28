//! Detection: the [`Analyzer`] "find" engine and its reconciliation layers.
//!
//! The [`Analyzer`] runs the enrichers and recognizers, then reconciles
//! their findings through the [`Layer`] stages, [`overlap`] and [`filter`],
//! each reshaping or pruning the working entity set. Its [`Analysis`] carries
//! the findings, and every entity the layers dropped, suppressed, with a
//! [`Dropped`] event saying why. Re-exported from
//! [`elide_detection`].
//!
//! Decorators that *wrap* a recognizer (escalation, resilience, chunking,
//! calibration) are a
//! recognition concern and live in [`crate::recognition`], not here.
//!
//! [`Analyzer`]: crate::detection::Analyzer
//! [`Analysis`]: elide_detection::Analysis
//! [`Dropped`]: elide_core::entity::audit::Dropped
//! [`Layer`]: elide_detection::Layer
//! [`overlap`]: elide_detection::overlap
//! [`filter`]: elide_detection::filter

#[doc(inline)]
pub use elide_detection::{Analysis, Analyzer, Layer, LayerOutput, filter, overlap};
