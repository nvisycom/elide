//! The result type a matched in-place analysis hands back ([`InPlaceAnalysis`]),
//! plus the [`BoxFuture`] alias the erased async methods return.

use std::future::Future;
use std::pin::Pin;

use crate::analysis::{ArtifactGroup, EntityGroup};

/// A boxed, pinned, `Send` future, the erased async return shape.
pub(crate) type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// What a matched in-place analysis produced: the boxed entities and the boxed
/// enrichment artifact (`Some` iff enriched). The batch path
/// ([`ErasedPipeline::analyze_streams`]) returns one per handle, already known to
/// match its modality.
///
/// [`ErasedPipeline::analyze_streams`]: super::erased::ErasedPipeline::analyze_streams
pub(crate) type PartAnalysis = (Box<dyn EntityGroup>, Option<Box<dyn ArtifactGroup>>);
