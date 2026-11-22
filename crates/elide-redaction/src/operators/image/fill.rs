//! [`Fill`]: cover the matched image region with a solid colour.

use elide_core::entity::Entity;
use elide_core::redaction::{LeakProfile, Operator};
use elide_core::{Component, Result};
use elide_image::modality::{Image, ImageData, ImageReplacement};
use elide_image::primitive::Color;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Cover the matched image region with a solid colour (black by default).
///
/// [`Cover`] in a colour of its own: a visible "something was here" box.
/// Contrast [`Blur`] / [`Pixelate`], which leave the region's content
/// partly perceptible.
///
/// [`Cover`]: crate::operators::Cover
/// [`Blur`]: crate::operators::Blur
/// [`Pixelate`]: crate::operators::Pixelate
#[derive(Debug, Clone, Copy, Component)]
#[component(name = "fill", version = "1.0.0")]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Fill {
    /// Fill colour the codec rasterizes over the region.
    color: Color,
}

impl Fill {
    /// Cover the region with `color`.
    pub fn new(color: Color) -> Self {
        Self { color }
    }
}

impl Default for Fill {
    fn default() -> Self {
        Self {
            color: Color::BLACK,
        }
    }
}

#[async_trait::async_trait]
impl Operator<Image> for Fill {
    fn leak_profile(&self) -> LeakProfile {
        // The original content is gone, but the region's position and
        // bounding box stay observable.
        LeakProfile::Partial
    }

    async fn anonymize(
        &self,
        _entity: &Entity<Image>,
        _data: &ImageData,
    ) -> Result<Option<ImageReplacement>> {
        Ok(Some(ImageReplacement::Covered { color: self.color }))
    }
}
