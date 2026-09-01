//! The analysis of a document: the two views its detection produces, and the
//! [`AnalyzedDocument`] that pairs them.
//!
//! Detection yields two parallel structures, keyed the same way (the body, plus
//! each container part by id) but carrying opposite things. The [`Report`] holds
//! *references* — each entity is a location into the body plus its audit trail,
//! editable before [`anonymize_with`](crate::Orchestrator::anonymize_with). The
//! [`ArtifactSet`] holds *content* — the enrichment those locations point into
//! (an image's OCR `Layout`, an audio clip's STT `Transcription`) — kept out
//! of the report so it stays references-only. Persist both across a review gap
//! and hand them to [`re_analyze`](crate::Orchestrator::re_analyze) to
//! re-recognize without re-enriching.
//!
//! The type-erased storage the two views share lives in [`group`]
//! ([`EntityGroup`] / [`ArtifactGroup`]); the per-modality reconstruction
//! [`registry`] and the public [`ReportDeserializer`] drive deserialization; and
//! the serde wire form of both views lives in [`serde`]. With the `schema`
//! feature, the report's hand-written `JsonSchema` lives in `schema`.

mod artifacts;
mod group;
mod registry;
mod report;
#[cfg(feature = "schema")]
mod schema;
mod serde;

pub use self::artifacts::ArtifactSet;
pub use self::group::{ArtifactGroup, EntityGroup};
pub(crate) use self::registry::ModalityRegistry;
pub use self::registry::ReportDeserializer;
pub use self::report::Report;
pub(crate) use self::report::{BodyReport, PartReport};

/// The result of analyzing a document: its findings and the enrichment that
/// produced them.
///
/// [`report`](Self::report) holds the detected entities (references into the
/// document). [`artifacts`](Self::artifacts) holds the enrichment content (OCR
/// `Layout`, STT `Transcription`) they were found in — kept apart because a
/// report is references only. Persist both across a review gap and hand them to
/// [`re_analyze`](crate::Orchestrator::re_analyze) to re-recognize without
/// re-enriching.
#[derive(Default)]
pub struct AnalyzedDocument {
    /// The detected entities, grouped by body and container part.
    pub report: Report,
    /// The enrichment artifact each group was detected in.
    pub artifacts: ArtifactSet,
}
