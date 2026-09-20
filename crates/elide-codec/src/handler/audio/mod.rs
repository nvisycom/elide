//! Audio modality: clip format handlers (WAV, MP3) that stream the whole
//! clip as one chunk and redact time ranges by editing decoded samples.
//!
//! Each format wraps the standalone [`elide_audio::AudioBuffer`] engine, which
//! holds the encoded bytes and decodes to samples only when a redaction is
//! applied on encode. The handlers here are thin adapters onto the codec's
//! `Handler`/`Loader` contracts, stamped out by [`impl_audio_handler!`].

#[cfg(feature = "internal_audio")]
mod macros;

#[cfg(feature = "mp3")]
mod mp3_handler;
#[cfg(feature = "wav")]
mod wav_handler;

// `*_format` is `pub` so the parent `handler` module re-exports it as the
// crate's public contract; the loader/handler pairs stay `pub(crate)`.
#[cfg(feature = "mp3")]
pub use self::mp3_handler::format as mp3_format;
#[cfg(feature = "wav")]
pub use self::wav_handler::format as wav_format;
