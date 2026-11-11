//! Codec adapters: the audio format handlers (WAV, MP3).
//!
//! Each format decodes into a one-stream [`Document`](elide_codec::Document):
//! the clip, streamed as one chunk and redacted by editing decoded samples, with
//! its tags stripped at decode under the hidden-content policy.
//!
//! Each format wraps this crate's [`AudioBuffer`] engine, which holds the
//! encoded bytes and decodes to samples only when a redaction is applied on
//! encode. The shared body is `AudioDocumentLoader` (`audio_loader`), the
//! `AudioStream` (`audio_stream`), and `AudioDocument` (`audio_document`); the
//! `impl_audio_handler!` macro stamps out each format's
//! id and lookup keys.
//!
//! [`AudioBuffer`]: crate::AudioBuffer

mod macros;

mod audio_document;
mod audio_loader;
mod audio_stream;

#[cfg(feature = "mp3")]
mod mp3_handler;
#[cfg(feature = "wav")]
mod wav_handler;

#[cfg(feature = "mp3")]
pub use self::mp3_handler::format as mp3_format;
#[cfg(feature = "wav")]
pub use self::wav_handler::format as wav_format;
