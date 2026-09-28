//! EXIF metadata read, strip, and field-level removal, plus the
//! [`ExifPolicy`] re-encode config and the [`ExifRecognizer`] that surfaces
//! privacy-relevant fields as `Entity<Metadata>`.
//!
//! Backed by `little_exif`, the one permissive pure-Rust crate that both reads
//! and edits EXIF for JPEG, PNG, and TIFF. Its parser can panic on malformed
//! input, so every call into it is wrapped in a panic guard: a panic becomes a
//! fail-closed error, never an aborted process, since this runs on the
//! redaction path where a crash mid-strip is unacceptable.
//!
//! The read/strip/transfer engine is `Source` (in `source`); the tag sets it
//! acts on are in `tags`; the entry point is [`ImageBuffer`], which opens the
//! image once and drives these helpers with a format it already knows.
//!
//! [`ExifPolicy`]: crate::exif::ExifPolicy
//! [`ExifRecognizer`]: crate::exif::ExifRecognizer
//! [`ImageBuffer`]: crate::ImageBuffer

mod entity;
mod policy;
mod recognizer;
mod source;
mod tags;

use std::panic::{AssertUnwindSafe, catch_unwind};

use elide_core::{Error, ErrorKind, Result};

pub use self::policy::ExifPolicy;
pub use self::recognizer::ExifRecognizer;
pub(crate) use self::source::Source;

/// Run `f`, turning a panic (little_exif can panic on malformed input) into a
/// fail-closed [`ErrorKind::MalformedInput`] instead of aborting.
fn guard<T>(f: impl FnOnce() -> T) -> Result<T> {
    catch_unwind(AssertUnwindSafe(f)).map_err(|_| {
        Error::new(
            ErrorKind::MalformedInput,
            "image metadata parser panicked on malformed input",
        )
    })
}
