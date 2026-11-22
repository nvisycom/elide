//! Image-modality operators: region treatments.
//!
//! [`Blur`], [`Pixelate`], and [`Fill`] obscure the pixels of an entity's
//! bounding box; [`Cover`] covers it in black. Re-exported from
//! [`operators`](super); this module is an internal grouping.
//!
//! [`Cover`]: crate::operators::Cover

mod blur;
mod fill;
mod pixelate;

pub use self::blur::Blur;
pub use self::fill::Fill;
pub use self::pixelate::Pixelate;
