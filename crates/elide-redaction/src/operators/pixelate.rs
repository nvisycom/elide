//! [`Pixelate`]: mosaic-pixelate the matched image region.

use elide_core::Result;
use elide_core::entity::Entity;
use elide_core::modality::image::{Image, ImageData, ImageReplacement};
use elide_core::operator::{LeakProfile, Operator, OperatorId};

/// Mosaic-pixelate the matched image region.
///
/// The "pixelated face" look: the region is reduced to coarse blocks. A
/// common compliance default — more obviously redacted than [`Blur`] while
/// still keeping the region's footprint visible. Contrast [`Blackbox`],
/// which hides it behind a solid fill.
///
/// [`Blur`]: super::Blur
/// [`Blackbox`]: super::Blackbox
#[derive(Debug, Clone, Copy)]
pub struct Pixelate {
    /// Side length of each mosaic block, in pixels. Larger blocks are
    /// coarser (and harder to reverse).
    block_size: u32,
}

impl Pixelate {
    /// Pixelate with the given mosaic block side length, in pixels.
    pub fn new(block_size: u32) -> Self {
        Self { block_size }
    }
}

impl Default for Pixelate {
    fn default() -> Self {
        // A coarse mosaic that destroys detail at typical document scales.
        Self { block_size: 16 }
    }
}

#[async_trait::async_trait]
impl Operator<Image> for Pixelate {
    fn id(&self) -> OperatorId {
        OperatorId::new("pixelate", "1.0.0")
    }

    fn leak_profile(&self) -> LeakProfile {
        // The region's position and bounding box stay observable, and a
        // fine mosaic can leak coarse structure.
        LeakProfile::Partial
    }

    async fn anonymize(
        &self,
        _entity: &Entity<Image>,
        _data: &ImageData,
    ) -> Result<ImageReplacement> {
        Ok(ImageReplacement::Pixelate {
            block_size: self.block_size,
        })
    }
}
