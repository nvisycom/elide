//! [`MockBackend`]: stand-in [`OcrBackend`] for tests, examples, and as a
//! default before a real backend is configured.

use elide_core::Result;
use elide_core::entity::audit::ModelEvent;
use elide_core::modality::image::LayoutBlock;

use super::{OcrBackend, OcrRequest, OcrResponse};

/// Mock OCR backend: returns a fixed set of blocks on every call.
///
/// Empty by default ([`new`](Self::new) / [`default`](Default::default)) — the
/// no-op stub examples and offline wiring rely on, recognizing nothing. Give it
/// canned blocks with [`with`](Self::with) to have every call enrich with the
/// same [`Layout`], for tests that need a real artifact to read back.
///
/// [`Layout`]: elide_core::modality::image::Layout
#[derive(Debug, Default, Clone)]
pub struct MockBackend {
    blocks: Vec<LayoutBlock>,
}

impl MockBackend {
    /// An empty mock backend: every call recognizes nothing.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A mock backend that returns `blocks` on every call, so the enricher
    /// stamps the same [`Layout`] onto each request.
    ///
    /// [`Layout`]: elide_core::modality::image::Layout
    #[must_use]
    pub fn with(blocks: Vec<LayoutBlock>) -> Self {
        Self { blocks }
    }
}

#[async_trait::async_trait]
impl OcrBackend for MockBackend {
    fn provenance(&self) -> ModelEvent {
        ModelEvent {
            name: "mock-ocr".into(),
            ..ModelEvent::default()
        }
    }

    async fn recognize(&self, _request: OcrRequest<'_>) -> Result<OcrResponse> {
        Ok(OcrResponse::new(self.blocks.clone()))
    }
}
