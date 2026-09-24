//! Codec adapters: the audio format handlers (WAV, MP3).
//!
//! Each handler streams the whole clip as one chunk and redacts time ranges by
//! editing decoded samples.
//!
//! Each format wraps this crate's [`AudioBuffer`] engine, which holds the
//! encoded bytes and decodes to samples only when a redaction is applied on
//! encode. The handlers here are thin adapters onto the [`elide_codec`]
//! `Handler`/`Loader` contracts, stamped out by the `impl_audio_handler!`
//! macro.
//!
//! [`AudioBuffer`]: crate::AudioBuffer

mod macros;

#[cfg(feature = "mp3")]
mod mp3_handler;
#[cfg(feature = "wav")]
mod wav_handler;

#[cfg(feature = "mp3")]
pub use self::mp3_handler::format as mp3_format;
#[cfg(feature = "wav")]
pub use self::wav_handler::format as wav_format;
