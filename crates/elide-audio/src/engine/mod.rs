//! The decode/redact/encode engine behind [`AudioBuffer`](crate::AudioBuffer).
//!
//! Compiled only when a format is enabled (the `_internal` marker). `duration`
//! and `redact` are shared across formats; `mp3` and `wav` are each gated on
//! their own format feature.

pub(crate) mod duration;
pub(crate) mod redact;

#[cfg(feature = "mp3")]
pub(crate) mod mp3;
#[cfg(feature = "wav")]
pub(crate) mod wav;
