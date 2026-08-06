//! Image-modality operators: region treatments.
//!
//! [`Blur`], [`Pixelate`], and [`Blackbox`] obscure the pixels of an
//! entity's bounding box. Re-exported from [`operators`](super); this
//! module is an internal grouping.

mod blackbox;
mod blur;
mod pixelate;

pub use self::blackbox::Blackbox;
pub use self::blur::Blur;
pub use self::pixelate::Pixelate;
