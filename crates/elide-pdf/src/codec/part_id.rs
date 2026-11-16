//! [`PartId`]: the container-local id of a blob a PDF surfaces, and how it
//! reads back on encode.

use std::fmt;

use elide_core::{Error, Result};

use crate::document::attachment::AttachmentId;
use crate::document::image::ImageId;

/// What a blob of a decoded PDF is: an embedded image (`img-N-G`), an
/// attachment (`att-N-G`), or (feature `render`) a scanned page (`page-N`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PartId {
    Image(ImageId),
    Attachment(AttachmentId),
    #[cfg(feature = "render")]
    Page(u32),
}

impl PartId {
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
        let part = if let Some(rest) = id.strip_prefix("img-") {
            object(rest).map(|(n, g)| Self::Image(ImageId::new(n, g)))
        } else if let Some(rest) = id.strip_prefix("att-") {
            object(rest).map(|(n, g)| Self::Attachment(AttachmentId::new(n, g)))
        } else {
            #[cfg(feature = "render")]
            let page = id
                .strip_prefix("page-")
                .and_then(|rest| rest.parse().ok())
                .map(Self::Page);
            #[cfg(not(feature = "render"))]
            let page = None;
            page
        };
        part.ok_or_else(|| Error::not_found(format!("{id} is not a part of this document")))
    }
}

impl fmt::Display for PartId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Image(id) => write!(f, "img-{}-{}", id.number, id.generation),
            Self::Attachment(id) => write!(f, "att-{}-{}", id.number, id.generation),
            #[cfg(feature = "render")]
            Self::Page(number) => write!(f, "page-{number}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_round_trips() {
        for id in [
            PartId::Image(ImageId::new(7, 3)),
            PartId::Attachment(AttachmentId::new(12, 0)),
            #[cfg(feature = "render")]
            PartId::Page(4),
        ] {
            assert_eq!(PartId::parse(&id.to_string()).unwrap(), id);
        }
        for id in ["body", "img-x-1", "page-", "att-1"] {
            assert!(PartId::parse(id).is_err(), "{id}");
        }
    }
}
