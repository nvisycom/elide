//! [`Hint<M>`]: a located piece of out-of-band context for recognition.

use std::fmt;

#[cfg(feature = "schema")]
use schemars::JsonSchema;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::Modality;

/// Located, typed piece of context a recognizer may treat as in-context
/// for a nearby value.
///
/// Out-of-band by nature: a hint is *not* a sub-span of the value it
/// informs — it lives elsewhere in the source (a table's column header, a
/// JSON object key, a log field name). So `location` points at where the
/// hint text actually sits, and `data` is the hint text itself. Carrying
/// the location (rather than a bare string) lets a confidence boost record
/// *which* hint lifted a score and *where* it came from — provenance a
/// review consumer can resolve back to the document.
///
/// Mirrors [`Entity`]'s `location` + `data` shape, so the same
/// serialization and lifting patterns apply.
///
/// [`Entity`]: crate::entity::Entity
// `Clone`/`Debug`/`PartialEq`/`Eq` bound on the field types `M::Location`/
// `M::Data`, not on the marker `M` (which is none of those), so they're
// impl'd below rather than derived.
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(bound = "M::Location: Serialize + for<'a> Deserialize<'a>, \
                   M::Data: Serialize + for<'a> Deserialize<'a>")
)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[cfg_attr(
    feature = "schema",
    schemars(bound = "M::Location: schemars::JsonSchema, M::Data: schemars::JsonSchema")
)]
pub struct Hint<M: Modality> {
    /// Where the hint text sits in the source (the header cell, the key).
    pub location: M::Location,
    /// The hint text itself (a column header, a field name).
    pub data: M::Data,
}

impl<M: Modality> Hint<M> {
    /// A hint whose text is `data`, located at `location`.
    pub fn new(location: M::Location, data: M::Data) -> Self {
        Self { location, data }
    }
}

impl<M: Modality> Clone for Hint<M> {
    fn clone(&self) -> Self {
        Self {
            location: self.location.clone(),
            data: self.data.clone(),
        }
    }
}

impl<M: Modality> fmt::Debug for Hint<M> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Hint")
            .field("location", &self.location)
            .field("data", &self.data)
            .finish()
    }
}

impl<M: Modality> PartialEq for Hint<M>
where
    M::Location: PartialEq,
    M::Data: PartialEq,
{
    fn eq(&self, other: &Self) -> bool {
        self.location == other.location && self.data == other.data
    }
}

impl<M: Modality> Eq for Hint<M>
where
    M::Location: Eq,
    M::Data: Eq,
{
}
