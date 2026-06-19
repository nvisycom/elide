//! WAV loader: validates the bytes parse as WAV, then wraps them.

use elide_core::modality::audio::Audio;
use elide_core::{Error, ErrorKind, Result};
use hound::WavReader;

use super::wav_handler::WavHandler;
use crate::Loader;
use crate::content::ContentData;

/// Loader that validates and wraps WAV content. Produces one
/// [`WavHandler`] per input.
#[derive(Debug)]
pub(crate) struct WavLoader;

impl Loader<Audio> for WavLoader {
    type Handler = WavHandler;

    async fn decode(&self, content: ContentData) -> Result<WavHandler> {
        let bytes = content.to_bytes();
        // Validate up front so a malformed clip fails at decode, not at
        // the first redaction.
        WavReader::new(std::io::Cursor::new(bytes.clone()))
            .map_err(|e| Error::new(ErrorKind::Validation, format!("not a valid WAV: {e}")))?;
        Ok(WavHandler::new(bytes))
    }
}
