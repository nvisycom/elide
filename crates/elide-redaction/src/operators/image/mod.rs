//! Image-modality operators: region treatments.
//!
//! [`Blur`] and [`Pixelate`] obscure the pixels of an entity's bounding box;
//! [`Cover`] covers it in a color. Re-exported from
//! [`operators`](super); this module is an internal grouping.
//!
//! [`Cover`]: crate::operators::Cover

mod blur;
mod pixelate;

pub use self::blur::Blur;
pub use self::pixelate::Pixelate;
