//! [`Hint<M>`]: a located piece of out-of-band context for recognition.

use std::fmt;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::Modality;

/// Located, out-of-band context a recognizer may treat as in-context for a
/// nearby value.
///
/// A hint is *not* a sub-span of the value it informs; it lives elsewhere in
/// the source (a table's column header, a JSON object key, a log field name).
/// It carries only the `location` where that context sits: a recognizer or
/// prompt reads the hint's text back from the source at that location, and a
/// confidence boost records *which* hint lifted a score and *where* it came
/// from — provenance a review consumer can resolve back to the document.
///
/// [`Entity`]: crate::entity::Entity
// `Clone`/`Debug`/`PartialEq`/`Eq` are bound on `M::Location`, not on the marker
// `M` (which is none of those), so they're impl'd below rather than derived.
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(bound = "M::Location: Serialize + for<'a> Deserialize<'a>")
)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(
    feature = "schema",
    schemars(
        bound = "M: schemars::JsonSchema, M::Location: schemars::JsonSchema",
        rename = "{M}Hint"
    )
)]
pub struct Hint<M: Modality> {
    /// Where the hint's context sits in the source (the header cell, the key).
    pub location: M::Location,
}

impl<M: Modality> Hint<M> {
    /// A hint located at `location`.
    pub fn new(location: M::Location) -> Self {
        Self { location }
    }
}

impl<M: Modality> Clone for Hint<M> {
    fn clone(&self) -> Self {
        Self {
            location: self.location.clone(),
        }
    }
}

impl<M: Modality> fmt::Debug for Hint<M> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Hint")
            .field("location", &self.location)
            .finish()
    }
}

impl<M: Modality> PartialEq for Hint<M>
where
    M::Location: PartialEq,
{
    fn eq(&self, other: &Self) -> bool {
        self.location == other.location
    }
}

impl<M: Modality> Eq for Hint<M> where M::Location: Eq {}

/// A [`Hint`] paired with the content at its location, the transient form a
/// recognizer sees.
///
/// A [`Hint`] itself carries only a location (all that a boost's audit trail
/// needs to record). During recognition, though, the enhancer must read the
/// hint's *content* — the header text, the field name — to match a context
/// keyword against it. The codec that surfaces a hint knows that content at
/// extraction time (it has both the value and its neighbouring header), so it
/// pairs the two here: `data` is the hint region as [`Data`], read through
/// [`as_text`] for matching, and `hint` carries the location recorded on any
/// boost it causes.
///
/// This is a recognition-time value only; it never serializes (it lives on a
/// [`Chunk`] and in a `RecognizerContext`, neither of which is a wire type), so
/// carrying [`Data`] here — unlike on the serialized [`Hint`] — puts no
/// serialization bound on the modality's data type.
///
/// [`Data`]: Modality::Data
/// [`as_text`]: super::TextRecognizable::as_text
/// [`Chunk`]: super::Chunk
pub struct ResolvedHint<M: Modality> {
    /// The located hint recorded on a boost's audit trail.
    pub hint: Hint<M>,
    /// The content at the hint's location, matched against context keywords.
    pub data: M::Data,
}

impl<M: Modality> ResolvedHint<M> {
    /// A hint at `location` whose content is `data`.
    pub fn new(location: M::Location, data: M::Data) -> Self {
        Self {
            hint: Hint::new(location),
            data,
        }
    }
}

impl<M: Modality> Clone for ResolvedHint<M> {
    fn clone(&self) -> Self {
        Self {
            hint: self.hint.clone(),
            data: self.data.clone(),
        }
    }
}

impl<M: Modality> fmt::Debug for ResolvedHint<M> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResolvedHint")
            .field("hint", &self.hint)
            .field("data", &self.data)
            .finish()
    }
}

impl<M: Modality> PartialEq for ResolvedHint<M>
where
    M::Location: PartialEq,
    M::Data: PartialEq,
{
    fn eq(&self, other: &Self) -> bool {
        self.hint == other.hint && self.data == other.data
    }
}

impl<M: Modality> Eq for ResolvedHint<M>
where
    M::Location: Eq,
    M::Data: Eq,
{
}
