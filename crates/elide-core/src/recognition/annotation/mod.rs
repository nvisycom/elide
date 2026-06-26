//! Caller-supplied region annotations that steer recognition.
//!
//! Two directions, two types. An [`Inclusion`] adds a candidate region
//! ("there may be an entity here"); recognizers that adjudicate it fold
//! it into detection. An [`Exclusion`] removes ("flag nothing here"); the
//! analyzer drops any entity overlapping it. Both carry only a
//! modality-native location plus the fields that make sense for their
//! direction.

mod annotations;
mod exclusion;
mod inclusion;

pub use self::annotations::Annotations;
pub use self::exclusion::Exclusion;
pub use self::inclusion::Inclusion;
