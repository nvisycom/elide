//! Modalities: the media entities live in.
//!
//! Re-exports the core modality vocabulary (the [`Modality`] trait, its
//! [`Text`]/`Image`/`Audio`/`Tabular` markers, [`Chunk`], the reader/writer
//! traits, …) from [`elide_core::modality`].
//!
//! [`Modality`]: elide_core::modality::Modality
//! [`Text`]: elide_core::modality::text::Text
//! [`Chunk`]: elide_core::modality::Chunk

#[doc(inline)]
pub use elide_core::modality::*;
