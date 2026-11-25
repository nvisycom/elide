//! [`Restorable`]: a report, a review, or an artifact set, which a
//! [`Restorer`](crate::Restorer) reads back against the digest it had when it
//! was written, and the fields that digest covers.

use elide_core::entity::audit::AuditHash;

use crate::PartId;

/// The digest of `value`: of its fields, its type's name first.
pub(crate) fn digest<T: Restorable>(value: &T) -> AuditHash {
    AuditHash::of_fields(fields(value))
}

/// The fields the digest of `value` covers: its type's name, so no two kinds
/// of value digest alike, then the value's own.
fn fields<T: Restorable>(value: &T) -> Vec<Vec<u8>> {
    let mut fields = vec![T::NAME.as_bytes().to_vec()];
    fields.extend(value.digest_fields());
    fields
}

/// The digest fields of `parts`, each a part's path, its modality, and what the
/// digest covers of it: in path order, so the order the parts were held in does
/// not count, each part's path and fields counted, so no two lists of parts
/// encode alike.
pub(crate) fn part_fields<'a>(
    parts: impl IntoIterator<Item = (&'a PartId, &'static str, Vec<Vec<u8>>)>,
) -> Vec<Vec<u8>> {
    let mut parts: Vec<_> = parts.into_iter().collect();
    parts.sort_unstable_by_key(|(id, ..)| *id);
    let mut fields = Vec::new();
    for (id, modality, body) in parts {
        let segments: Vec<&str> = id.segments().collect();
        fields.push((segments.len() as u64).to_le_bytes().to_vec());
        fields.extend(segments.iter().map(|segment| segment.as_bytes().to_vec()));
        fields.push(modality.as_bytes().to_vec());
        fields.push((body.len() as u64).to_le_bytes().to_vec());
        fields.extend(body);
    }
    fields
}

/// A value a [`Restorer`](crate::Restorer) reads back, checked against its
/// digest: a [`Report`], a [`Review`], or an [`ArtifactSet`].
///
/// Implemented only by those three.
///
/// [`Report`]: crate::Report
/// [`Review`]: crate::Review
/// [`ArtifactSet`]: crate::ArtifactSet
pub trait Restorable: private::Sealed {}

impl<T: private::Sealed> Restorable for T {}

pub(crate) mod private {
    use elide_core::Result;
    use serde::Deserializer;

    use super::super::Restorer;

    /// What a [`Restorable`](super::Restorable) provides: the fields its
    /// digest covers, and how a registry parses it. Implemented beside the
    /// registry's parsers.
    pub trait Sealed: Sized {
        /// What the value is called in an error.
        const NAME: &'static str;

        /// The fields the value's digest covers, in a canonical order.
        fn digest_fields(&self) -> Vec<Vec<u8>>;

        /// Parse the value through `registry`, without checking its digest.
        fn parse<'de, D: Deserializer<'de>>(registry: &Restorer, de: D) -> Result<Self>;
    }
}
