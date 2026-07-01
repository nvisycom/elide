//! MP3 loader: rejects clips with more than two channels (the LAME
//! encoder handles only mono and stereo), then wraps the bytes.

use elide_core::modality::audio::Audio;
use elide_core::{Error, ErrorKind, Result};

use super::mp3_codec::probe_channels;
use super::mp3_handler::Mp3Handler;
use crate::Loader;
use crate::content::ContentData;

/// Loader that validates channel count and wraps MP3 content. Produces
/// one [`Mp3Handler`] per input.
#[derive(Debug)]
pub(crate) struct Mp3Loader;

#[async_trait::async_trait]
impl Loader<Audio> for Mp3Loader {
    type Handler = Mp3Handler;

    async fn decode(&self, content: ContentData) -> Result<Mp3Handler> {
        let bytes = content.to_bytes();
        let channels = probe_channels(&bytes)?;
        if channels > 2 {
            return Err(Error::new(
                ErrorKind::Validation,
                format!("MP3 has {channels} channels; only mono and stereo are supported"),
            ));
        }
        Ok(Mp3Handler::new(bytes))
    }
}
