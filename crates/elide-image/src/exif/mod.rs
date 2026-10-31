//! EXIF metadata read, strip, and field-level removal. Each surfaced field is
//! labelled with what it is (a location, a person, a device, a time), so the
//! format-neutral field recognizer (`elide-detection`'s `FieldRecognizer`)
//! turns it into an `Entity<Metadata>`.
//!
//! Backed by `little_exif`, the one permissive pure-Rust crate that both reads
//! and edits EXIF for JPEG, PNG, and TIFF. Its parser can panic on malformed
//! input, so every call into it is wrapped in a panic guard: a panic becomes a
//! fail-closed error, never an aborted process, since this runs on the
//! redaction path where a crash mid-strip is unacceptable.
//!
//! The read/strip/transfer engine is `Source` (in `source`); the tag sets it
//! acts on are in `tags`; the scrub of the metadata blocks outside EXIF (XMP,
//! IPTC, comments, text chunks) is in `container`; the entry point is
//! [`ImageBuffer`], which opens the image once and drives these helpers with a
//! format it already knows.
//!
//! [`ImageBuffer`]: crate::ImageBuffer

mod container;
mod entity;
mod source;
mod tags;
mod value;

use std::panic::{AssertUnwindSafe, catch_unwind};

use elide_core::{Error, ErrorKind, Result};

pub(crate) use self::container::Scrubbed;
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
