//! Audio modality: clip format handlers (WAV, MP3) that stream the whole
//! clip as one chunk and redact time ranges by editing decoded samples.
//!
//! Each format holds the encoded bytes and decodes to samples only when a
//! redaction needs it: WAV via `hound`, MP3 via `symphonia` + LAME. The
//! shared [`redact`] step silences or removes a `[start_ms, end_ms)` span
//! of the interleaved sample buffer; the handler then re-encodes.
//! Replacements use [`AudioReplacement`] (silence, remove).
//!
//! [`AudioReplacement`]: elide_core::modality::audio::AudioReplacement

#[cfg(any(feature = "wav", feature = "mp3"))]
mod duration;
#[cfg(any(feature = "wav", feature = "mp3"))]
mod redact;

#[cfg(feature = "mp3")]
mod mp3_codec;
#[cfg(feature = "mp3")]
mod mp3_handler;
#[cfg(feature = "mp3")]
mod mp3_loader;
#[cfg(feature = "wav")]
mod wav_handler;
#[cfg(feature = "wav")]
mod wav_loader;

// `*_format` is `pub` so the parent `handler` module re-exports it as the
// crate's public contract; the loader/handler pairs stay `pub(crate)`.
#[cfg(feature = "mp3")]
pub use self::mp3_handler::format as mp3_format;
#[cfg(feature = "wav")]
pub use self::wav_handler::format as wav_format;
