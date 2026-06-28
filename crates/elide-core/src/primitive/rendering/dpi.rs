//! [`Dpi`]: a dots-per-inch resolution for rasterizing vector content.

#[cfg(feature = "schema")]
use schemars::JsonSchema;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Dots-per-inch resolution for rasterizing vector content.
///
/// Used e.g. for rendering PDF pages to images for OCR.
///
/// PDF coordinates are in points (1 pt = 1/72 in), so
/// [`scale_factor`] gives the multiplier from points to
/// pixels at this resolution.
///
/// [`scale_factor`]: Self::scale_factor
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[cfg_attr(feature = "schema", schemars(transparent))]
pub struct Dpi(u16);

impl Dpi {
    /// The resolution OCR engines expect: 300 DPI. Lower loses small text;
    /// higher mostly bloats memory for little accuracy gain.
    pub const OCR: Self = Self(300);
    /// Points per inch in PDF/PostScript coordinates (1 pt = 1/72 in), the
    /// baseline [`scale_factor`] measures against.
    ///
    /// [`scale_factor`]: Self::scale_factor
    const POINTS_PER_INCH: u16 = 72;

    /// Create a DPI value from a raw `u16`.
    pub const fn new(value: u16) -> Self {
        Self(value)
    }

    /// The raw numeric value.
    pub const fn value(self) -> u16 {
        self.0
    }

    /// Scale factor relative to PDF points (1 pt = 1/72 in).
    pub fn scale_factor(self) -> f32 {
        self.0 as f32 / Self::POINTS_PER_INCH as f32
    }
}

impl Default for Dpi {
    fn default() -> Self {
        Self::OCR
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_ocr() {
        assert_eq!(Dpi::default(), Dpi::OCR);
        assert_eq!(Dpi::OCR.value(), 300);
    }

    #[test]
    fn scale_factor_is_relative_to_points() {
        // 72 DPI matches PDF points: no scaling.
        assert_eq!(Dpi::new(72).scale_factor(), 1.0);
        // 300 DPI scales points up by 300/72.
        assert_eq!(Dpi::OCR.scale_factor(), 300.0 / 72.0);
    }
}
