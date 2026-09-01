//! The result types the erased pipeline hands back: [`AnalyzeOutcome`] (owned
//! handle, accept-or-reject) and [`InPlaceAnalysis`] (borrowed handle), plus the
//! [`BoxFuture`] alias the erased async methods return.

use std::any::TypeId;
use std::future::Future;
use std::pin::Pin;

use elide_codec::UntypedDocumentHandle;
#[cfg(feature = "usage")]
use elide_core::recognition::Usage;

use crate::analysis::{ArtifactGroup, EntityGroup};

/// A boxed, pinned, `Send` future — the erased async return shape.
pub(crate) type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// What a matched in-place analysis produced: the boxed entities and the boxed
/// enrichment artifact (`Some` iff the payload was enriched — even to an empty
/// artifact — so an un-enriched payload persists nothing); the whole result is
/// `None` when the pipeline's modality did not match the handle. Under the
/// `usage` feature it also carries the per-component [`Usage`] the analysis
/// recorded. The result of [`ErasedPipeline::analyze_in_place`].
///
/// [`ErasedPipeline::analyze_in_place`]: super::erased::ErasedPipeline::analyze_in_place
#[cfg(feature = "usage")]
pub(crate) type InPlaceAnalysis = Option<(
    Box<dyn EntityGroup>,
    Option<Box<dyn ArtifactGroup>>,
    Vec<Usage>,
)>;
/// What a matched in-place analysis produced: the boxed entities and the boxed
/// enrichment artifact (`Some` iff enriched); the whole result is `None` when
/// the pipeline's modality did not match the handle. The result of
/// [`ErasedPipeline::analyze_in_place`].
///
/// [`ErasedPipeline::analyze_in_place`]: super::erased::ErasedPipeline::analyze_in_place
#[cfg(not(feature = "usage"))]
pub(crate) type InPlaceAnalysis = Option<(Box<dyn EntityGroup>, Option<Box<dyn ArtifactGroup>>)>;

/// The result of offering a decoded handle to a pipeline for analysis: the
/// pipeline either accepts it (its modality matched) and returns the
/// detected entities boxed by modality, or rejects it (a different
/// modality) and hands the handle back for another pipeline to try.
pub(crate) enum AnalyzeOutcome {
    /// Modality matched: the matched modality's `TypeId`, the retained
    /// handle, its boxed `Vec<Entity<M>>` (recoverable as that modality), and
    /// the per-component [`Usage`] the analysis recorded.
    Accepted {
        modality: TypeId,
        handle: UntypedDocumentHandle,
        entities: Box<dyn EntityGroup>,
        /// The enrichment artifact, `Some` iff the payload was enriched (even to
        /// an empty artifact); `None` for an un-enriched payload, which persists
        /// nothing.
        artifact: Option<Box<dyn ArtifactGroup>>,
        #[cfg(feature = "usage")]
        usage: Vec<Usage>,
    },
    /// Not this pipeline's modality; the undecoded handle is returned.
    Rejected(UntypedDocumentHandle),
}
