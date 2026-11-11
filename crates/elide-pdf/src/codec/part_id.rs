//! [`PartId`]: the container-local id of a blob a PDF surfaces, and how it
//! reads back on encode.

use std::fmt;

use crate::document::attachment::AttachmentId;
use crate::document::image::ImageId;

/// What a blob of a decoded PDF is: an embedded image (`img-N-G`), an
/// attachment (`att-N-G`), or a scanned page (`page-N`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PartId {
    Image(ImageId),
    Attachment(AttachmentId),
    Page(u32),
}

impl PartId {
    /// The blob `id` names, or `None` for an id that is no blob (the body
    /// stream's).
    pub(super) fn parse(id: &str) -> Option<Self> {
        let object = |rest: &str| -> Option<(u32, u16)> {
            let (number, generation) = rest.split_once('-')?;
            Some((number.parse().ok()?, generation.parse().ok()?))
        };
        if let Some(rest) = id.strip_prefix("img-") {
            let (number, generation) = object(rest)?;
            return Some(Self::Image(ImageId::new(number, generation)));
        }
        if let Some(rest) = id.strip_prefix("att-") {
            let (number, generation) = object(rest)?;
            return Some(Self::Attachment(AttachmentId::new(number, generation)));
        }
        id.strip_prefix("page-")?.parse().ok().map(Self::Page)
    }
}

impl fmt::Display for PartId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Image(id) => write!(f, "img-{}-{}", id.number, id.generation),
            Self::Attachment(id) => write!(f, "att-{}-{}", id.number, id.generation),
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
            PartId::Page(4),
        ] {
            assert_eq!(PartId::parse(&id.to_string()), Some(id));
        }
        assert_eq!(PartId::parse("body"), None);
        assert_eq!(PartId::parse("img-x-1"), None);
    }
}
