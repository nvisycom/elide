//! Modalities: the media entities live in.
//!
//! Re-exports the core modality vocabulary (the [`Modality`] trait, its
//! [`Text`]/`Tabular`/`Metadata` markers, [`Chunk`], the reader/writer traits,
//! …) from [`elide_core::modality`], plus the `image` and `audio` modalities
//! from their own crates (`elide_image`, `elide_audio`, under the `image` /
//! `audio` features) so the whole modality vocabulary reads as one namespace.
//!
//! [`Modality`]: elide_core::modality::Modality
//! [`Text`]: elide_core::modality::text::Text
//! [`Chunk`]: elide_core::modality::Chunk

/// The audio modality: [`Audio`](elide_audio::modality::Audio) and its payload/location/
/// replacement/transcription types.
#[cfg(feature = "audio")]
#[doc(inline)]
pub use elide_audio::modality as audio;
#[doc(inline)]
pub use elide_core::modality::*;
/// The image modality: [`Image`](elide_image::modality::Image) and its payload/location/
/// replacement/layout types.
#[cfg(feature = "image")]
#[doc(inline)]
pub use elide_image::modality as image;
