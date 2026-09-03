//! [`Blackbox`]: cover the matched image region with a solid fill.

use elide_core::Result;
use elide_core::entity::Entity;
use elide_core::modality::image::{Image, ImageData, ImageReplacement};
use elide_core::primitive::Color;
use elide_core::redaction::{LeakProfile, Operator, OperatorId};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Cover the matched image region with a solid color (black by default).
///
/// The legal-redaction look: a visible "something was here" box. Distinct
/// from [`Erase`], which removes the region structurally, [`Blackbox`]
/// keeps it present but opaque. Contrast [`Blur`] / [`Pixelate`], which
/// leave the region's content partly perceptible.
///
/// [`Erase`]: crate::operators::Erase
/// [`Blur`]: crate::operators::Blur
/// [`Pixelate`]: crate::operators::Pixelate
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
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
