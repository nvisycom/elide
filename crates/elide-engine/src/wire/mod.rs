//! How a [`Report`], a [`Review`], and an [`ArtifactSet`] cross a process
//! boundary: their serde wire forms, the digest each is read back against,
//! and the [`Restorer`] that reads them, routing each part to its registered
//! modality.
//!
//! A wire form drops each part's concrete modality type, tagging the part
//! with its modality name instead, and deserialization is not object-safe, so
//! the `restorer` keeps a parser per registered modality, and the seed
//! visitors in `serde` replay each buffered part through it. With the `schema`
//! feature, the hand-written `JsonSchema` of each wire form lives in `schema`.
//!
//! [`Report`]: crate::Report
//! [`Review`]: crate::Review
//! [`ArtifactSet`]: crate::ArtifactSet

mod digest;
mod restorer;
#[cfg(feature = "schema")]
mod schema;
mod serde;

pub use self::digest::Restorable;
pub(crate) use self::digest::{digest, part_fields};
pub use self::restorer::Restorer;
pub(crate) use self::serde::Erased;
