//! The result type a matched in-place analysis hands back ([`InPlaceAnalysis`]),
//! plus the [`BoxFuture`] alias the erased async methods return.

use std::future::Future;
use std::pin::Pin;

use crate::analysis::{ArtifactGroup, EntityGroup};

/// A boxed, pinned, `Send` future, the erased async return shape.
pub(crate) type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// What a matched in-place analysis produced: the boxed entities and the boxed
/// enrichment artifact (`Some` iff enriched); the whole result is `None` when
/// the pipeline's modality did not match the stream. The result of
/// [`ErasedPipeline::analyze_stream`].
///
/// [`ErasedPipeline::analyze_stream`]: super::erased::ErasedPipeline::analyze_stream
pub(crate) type InPlaceAnalysis = Option<(Box<dyn EntityGroup>, Option<Box<dyn ArtifactGroup>>)>;
