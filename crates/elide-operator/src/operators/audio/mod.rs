//! Audio-modality operators: interval treatments.
//!
//! [`Silence`] mutes and [`Beep`] overlays a tone across an entity's time
//! range. Re-exported from [`operators`](super); this module is an
//! internal grouping.

mod beep;
mod silence;

pub use self::beep::Beep;
pub use self::silence::Silence;
