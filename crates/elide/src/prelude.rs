//! The common imports for assembling a pipeline.
//!
//! A `use elide::prelude::*;` brings the engines ([`Analyzer`],
//! [`Anonymizer`], [`Deanonymizer`], and — with `codec` — `Orchestrator`
//! and the `FormatRegistry` that decodes documents),
//! the error types, the [`Recognizer`]/[`Operator`]/[`Modality`] contracts
//! and the [`Scope`] they run against, the reconciliation [`Layer`]s with
//! their usual strategies, and the common vocabulary — the modality markers
//! (`Text`, and the feature-gated `Image`/`Audio`/`Tabular`), `Entity`,
//! `LabelRef`, the [`builtins`] label set, `Confidence`/`ConfidenceThreshold`,
//! and `Language`/`LanguageTag`. The [`operators`] module comes along too, so
//! `prelude::operators::*` reaches the concrete operators without the longer
//! path. The concrete recognizers and backends are left out — they vary per
//! use case and a few names collide — so import those from [`recognition`].
//!
//! [`Analyzer`]: crate::detection::Analyzer
//! [`Anonymizer`]: crate::redaction::Anonymizer
//! [`Deanonymizer`]: crate::redaction::Deanonymizer
//! [`Recognizer`]: crate::recognition::Recognizer
//! [`Operator`]: crate::redaction::Operator
//! [`Modality`]: crate::modality::Modality
//! [`Scope`]: crate::recognition::Scope
//! [`Layer`]: crate::detection::Layer
//! [`builtins`]: crate::entity::builtins
//! [`operators`]: crate::redaction::operators
//! [`recognition`]: crate::recognition

#[doc(no_inline)]
pub use async_trait::async_trait;
#[cfg(feature = "codec")]
#[doc(no_inline)]
pub use elide_codec::FormatRegistry;
#[doc(no_inline)]
pub use elide_core::entity::{Entity, LabelRef, builtins};
#[doc(no_inline)]
pub use elide_core::modality::Modality;
#[cfg(feature = "audio")]
#[doc(no_inline)]
pub use elide_core::modality::audio::Audio;
#[cfg(feature = "image")]
#[doc(no_inline)]
pub use elide_core::modality::image::Image;
#[cfg(feature = "tabular")]
#[doc(no_inline)]
pub use elide_core::modality::tabular::Tabular;
#[doc(no_inline)]
pub use elide_core::modality::text::Text;
#[doc(no_inline)]
pub use elide_core::primitive::{Confidence, ConfidenceThreshold, Language, LanguageTag};
#[doc(no_inline)]
pub use elide_core::recognition::{Recognizer, Scope};
#[doc(no_inline)]
pub use elide_core::{Error, ErrorKind, Result};
#[doc(no_inline)]
pub use elide_detection::{
    Analyzer, Layer,
    calibrate::CalibrateLayer,
    filter::FilterLayer,
    reconcile::{
        Merging, ReconcileLayer, Structural,
        group::{CrossLabel, SameLabel},
        scoring::MaxConfidence,
    },
};
#[cfg(feature = "codec")]
#[doc(no_inline)]
pub use elide_orchestration::{Orchestrator, Report};
#[doc(no_inline)]
pub use elide_redaction::{Anonymizer, Deanonymizer, Operator, operators};
