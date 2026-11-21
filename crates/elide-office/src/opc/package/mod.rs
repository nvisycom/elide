//! The package layer: the zip container and its parts, the content-types
//! manifest and the relationships that type and connect them, and the
//! classification of every part into a [`PartKind`](kind::PartKind).

pub(crate) mod archive;
pub(crate) mod content_types;
pub(crate) mod index;
pub(crate) mod kind;
pub(crate) mod package_xml;
pub(super) mod part;
pub(crate) mod relationships;
pub(super) mod store;
