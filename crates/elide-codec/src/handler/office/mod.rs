//! The shared OOXML codec layer: helpers common to the docx, pptx, and xlsx
//! adapters over the [`elide_office`] engine.
//!
//! The three OOXML formats decode to the same `Package`-backed engine and adapt
//! to the codec's [`Handler`](crate::Handler) contract the same way; this module
//! holds the pieces that would otherwise be copy-pasted across them: the error
//! mapping, and the [`props`] document-property (`docProps/*`) sub-part handler.

pub(crate) mod props;

use elide_core::{Error, ErrorKind};

pub use self::props::{DocPropsRecognizer, format as docprops_format};

/// The registry hint (pseudo-extension) the OOXML `#docprops` sub-part is decoded
/// with; every OOXML container surfaces its property parts under it.
pub(crate) fn docprops_hint() -> &'static str {
    self::props::PROPS_HINT
}

/// Map an [`elide_office`] error into the codec's error type.
///
/// A malformed package (bad zip, missing part, invalid XML) is the caller's
/// [`MalformedInput`](ErrorKind::MalformedInput); everything else, an unsafe
/// rewrite or any future engine error, is a [`Processing`](ErrorKind::Processing)
/// failure. The wildcard is load-bearing: `elide_office::ErrorKind` is
/// `#[non_exhaustive]`, and a new variant defaults to `Processing`.
pub(crate) fn office_error(err: elide_office::Error) -> Error {
    use elide_office::ErrorKind::{InvalidArchive, InvalidPackage, InvalidXml};
    let kind = match err.kind() {
        InvalidArchive | InvalidPackage | InvalidXml => ErrorKind::MalformedInput,
        _ => ErrorKind::Processing,
    };
    Error::new(kind, err.to_string())
}
