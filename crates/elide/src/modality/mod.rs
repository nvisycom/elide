//! Modalities: the media entities live in.
//!
//! Re-exports the core modality vocabulary (the [`Modality`] trait, its
//! [`Text`]/`Image`/`Audio`/`Tabular` markers, [`Chunk`], the reader/writer
//! traits, …) from [`elide_core::modality`], and adds [`Erasable`] — the
//! capability the toolkit's modality-agnostic [`Erase`] operator binds to.
//!
//! [`Modality`]: elide_core::modality::Modality
//! [`Text`]: elide_core::modality::text::Text
//! [`Chunk`]: elide_core::modality::Chunk
//! [`Erase`]: crate::redaction::operators::Erase

mod erasable;

#[doc(inline)]
pub use elide_core::modality::*;

pub use self::erasable::Erasable;
