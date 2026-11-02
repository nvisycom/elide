//! The decode/redact/encode engine behind [`AudioBuffer`](crate::AudioBuffer).
//!
//! Compiled only when a format is enabled (the `_internal` marker). `duration`,
//! `redact`, and the metadata strip in `tags` are shared across formats; `mp3`
//! and `wav` are each gated on their own format feature.

pub(crate) mod duration;
pub(crate) mod redact;
pub(crate) mod tags;

#[cfg(feature = "mp3")]
pub(crate) mod mp3;
#[cfg(feature = "wav")]
pub(crate) mod wav;
