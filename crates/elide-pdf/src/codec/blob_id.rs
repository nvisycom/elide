//! [`BlobId`]: the container-local id of a blob a PDF surfaces, and how it
//! reads back on encode.

use std::fmt;

use elide_core::{Error, Result};

use crate::document::attachment::AttachmentId;
use crate::document::image::ImageId;

/// What a blob of a decoded PDF is: an embedded image (`img-N-G`) or an
/// attachment (`att-N-G`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BlobId {
    Image(ImageId),
    Attachment(AttachmentId),
}

impl BlobId {
    /// The blob `id` names.
    ///
    /// # Errors
    ///
    /// [`NotFound`] for an id that names no blob of a PDF: a part the
    /// document never surfaced is never silently skipped on encode.
    ///
    /// [`NotFound`]: elide_core::ErrorKind::NotFound
    pub(super) fn parse(id: &str) -> Result<Self> {
        let object = |rest: &str| -> Option<(u32, u16)> {
            let (number, generation) = rest.split_once('-')?;
            Some((number.parse().ok()?, generation.parse().ok()?))
        };
        let blob = if let Some(rest) = id.strip_prefix("img-") {
            object(rest).map(|(n, g)| Self::Image(ImageId::new(n, g)))
        } else if let Some(rest) = id.strip_prefix("att-") {
            object(rest).map(|(n, g)| Self::Attachment(AttachmentId::new(n, g)))
        } else {
            None
        };
        blob.ok_or_else(|| Error::not_found(format!("{id} is not a blob of this document")))
    }
}

impl fmt::Display for BlobId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Image(id) => write!(f, "img-{}-{}", id.number, id.generation),
            Self::Attachment(id) => write!(f, "att-{}-{}", id.number, id.generation),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_round_trips() {
        for id in [
            BlobId::Image(ImageId::new(7, 3)),
            BlobId::Attachment(AttachmentId::new(12, 0)),
        ] {
            assert_eq!(BlobId::parse(&id.to_string()).unwrap(), id);
        }
        for id in ["body", "img-x-1", "page-1", "att-1"] {
            assert!(BlobId::parse(id).is_err(), "{id}");
        }
    }
}
