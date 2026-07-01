//! [`Blackbox`]: cover the matched image region with a solid fill.

use elide_core::Result;
use elide_core::entity::Entity;
use elide_core::modality::image::{Image, ImageData, ImageReplacement};
use elide_core::operator::{LeakProfile, Operator, OperatorId};
use elide_core::primitive::Color;

/// Cover the matched image region with a solid color (black by default).
///
/// The legal-redaction look: a visible "something was here" box. Distinct
/// from [`Erase`], which removes the region structurally — `Blackbox`
/// keeps it present but opaque. Contrast [`Blur`] / [`Pixelate`], which
/// leave the region's content partly perceptible.
///
/// [`Erase`]: super::Erase
/// [`Blur`]: super::Blur
/// [`Pixelate`]: super::Pixelate
#[derive(Debug, Clone, Copy)]
pub struct Blackbox {
    /// Fill color the codec rasterizes over the region.
    color: Color,
}

impl Blackbox {
    /// Cover the region with `color`.
    pub fn new(color: Color) -> Self {
        Self { color }
    }
}

impl Default for Blackbox {
    fn default() -> Self {
        Self {
            color: Color::BLACK,
        }
    }
}

#[async_trait::async_trait]
impl Operator<Image> for Blackbox {
    fn id(&self) -> OperatorId {
        OperatorId::new("blackbox", "1.0.0")
    }

    fn leak_profile(&self) -> LeakProfile {
        // The original content is gone, but the region's position and
        // bounding box stay observable.
        LeakProfile::Partial
    }

    async fn anonymize(
        &self,
        _entity: &Entity<Image>,
        _data: &ImageData,
    ) -> Result<ImageReplacement> {
        Ok(ImageReplacement::Block { color: self.color })
    }
}
