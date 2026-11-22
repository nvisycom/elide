//! Audio-modality operators: interval treatments.
//!
//! [`Beep`] overlays a tone across an entity's time range; [`Cover`] silences
//! it. Re-exported from [`operators`](super); this module is an internal
//! grouping.
//!
//! [`Cover`]: crate::operators::Cover

mod beep;

pub use self::beep::Beep;
