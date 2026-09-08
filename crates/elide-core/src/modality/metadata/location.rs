//! [`MetadataLocation`]: a metadata field addressed by key.

use std::cmp::Ordering;

use hipstr::HipStr;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::modality::{ModalityLocation, Overlap};

/// A metadata field, addressed by its key within a metadata modality.
///
/// The owning modality (each crate's own marker, e.g. an image crate's
/// `ExifMetadata` or a filesystem crate's `FileMetadata`) already says *which*
/// scheme the field belongs to, so the location only needs the field's
/// [`key`](Self::key) within it (`"GPSLatitude"`, `"modified"`,
/// `"xattr:com.apple.metadata:kMDItemWhereFroms"`). A field is atomic: it has no
/// sub-extent, so two locations either address the same field or are unrelated.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct MetadataLocation {
    /// The field's identifier within its modality (e.g. `"GPSLatitude"`).
    #[cfg_attr(feature = "schema", schemars(with = "String"))]
    pub key: HipStr<'static>,
}

impl MetadataLocation {
    /// A field addressed by `key`.
    pub fn new(key: impl Into<HipStr<'static>>) -> Self {
        Self { key: key.into() }
    }
}

impl ModalityLocation for MetadataLocation {
    fn overlap(&self, other: &Self) -> Overlap {
        // A field is atomic and indivisible: it either is the same field
        // (reflexive containment, per the trait contract `a.overlap(a) ==
        // Contains`) or is unrelated. There is no partial crossing.
        if self.key == other.key {
            Overlap::Contains
        } else {
            Overlap::Disjoint
        }
    }

    fn union(&self, other: &Self) -> Option<Self> {
        // Two distinct fields can't coalesce into one redactable span; the same
        // field unions to itself.
        (self.key == other.key).then(|| self.clone())
    }

    fn span_cmp(&self, _other: &Self) -> Ordering {
        // Every field has the same (nil) extent, so none is "more specific".
        Ordering::Equal
    }

    fn position_cmp(&self, other: &Self) -> Ordering {
        // A stable, deterministic order for a single-pass apply. Fields have no
        // inherent position in the document, so order by key.
        self.key.cmp(&other.key)
    }

    fn hash(&self) -> Vec<u8> {
        // Length-prefix the key so the digest is unambiguous.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&(self.key.len() as u64).to_le_bytes());
        bytes.extend_from_slice(self.key.as_bytes());
        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_key_contains_itself_others_disjoint() {
        let a = MetadataLocation::new("GPSLatitude");
        assert!(matches!(a.overlap(&a), Overlap::Contains));
        assert!(a.overlaps(&a));

        let b = MetadataLocation::new("Make");
        assert!(matches!(a.overlap(&b), Overlap::Disjoint));
        assert!(!a.overlaps(&b));
    }

    #[test]
    fn union_only_within_the_same_field() {
        let a = MetadataLocation::new("GPSLatitude");
        assert_eq!(a.union(&a), Some(a.clone()));
        let b = MetadataLocation::new("Make");
        assert_eq!(a.union(&b), None);
    }

    #[test]
    fn position_is_by_key() {
        let a = MetadataLocation::new("Make");
        let b = MetadataLocation::new("Model");
        assert_eq!(a.position_cmp(&b), Ordering::Less);
    }

    #[test]
    fn hash_distinguishes_keys() {
        let a = MetadataLocation::new("Make");
        let b = MetadataLocation::new("Model");
        assert_ne!(a.hash(), b.hash());
    }
}
