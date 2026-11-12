//! Raw content bytes: the payload a codec decodes and re-encodes.

use std::borrow::Cow;
use std::fmt;
use std::path::Path;

use bytes::Bytes;
use elide_core::{Error, Result};

/// Raw content bytes, with the filename and content type a caller knows.
///
/// The data the codec moves around: the bytes, plus the caller-supplied
/// hints a registry can use to resolve a format: an original [`filename`]
/// and a declared [`content_type`]. The metadata fields are optional; the
/// caller supplies whatever it knows. Helpers cover byte access and UTF-8
/// [`decode`].
///
/// [`filename`]: ContentData::filename
/// [`content_type`]: ContentData::content_type
/// [`decode`]: ContentData::decode
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentData {
    data: Bytes,
    filename: Option<Cow<'static, str>>,
    content_type: Option<Cow<'static, str>>,
}

impl ContentData {
    /// Wrap raw bytes, with no metadata.
    #[must_use]
    pub fn new(data: Bytes) -> Self {
        Self {
            data,
            filename: None,
            content_type: None,
        }
    }

    /// Wrap UTF-8 text, with no metadata.
    #[must_use]
    pub fn from_text(text: impl Into<String>) -> Self {
        Self::new(Bytes::from(text.into().into_bytes()))
    }

    /// Attach an original filename (e.g. `"report.txt"`). The registry
    /// can derive a format-resolution [`extension`] from it.
    ///
    /// [`extension`]: Self::extension
    #[must_use]
    pub fn with_filename(mut self, filename: impl Into<Cow<'static, str>>) -> Self {
        self.filename = Some(filename.into());
        self
    }

    /// Attach a declared MIME content type (e.g. `"text/plain"`), as
    /// supplied by an HTTP `Content-Type` header or an explicit caller
    /// hint.
    #[must_use]
    pub fn with_content_type(mut self, content_type: impl Into<Cow<'static, str>>) -> Self {
        self.content_type = Some(content_type.into());
        self
    }

    /// Decode the bytes as UTF-8 text.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput)
    /// when the bytes are not valid UTF-8.
    pub fn decode(&self) -> Result<String> {
        String::from_utf8(self.data.to_vec())
            .map_err(|e| Error::malformed(format!("invalid UTF-8: {e}")))
    }

    /// The original filename, if one was attached.
    #[must_use]
    pub fn filename(&self) -> Option<&str> {
        self.filename.as_deref()
    }

    /// The declared MIME content type, if one was attached.
    #[must_use]
    pub fn content_type(&self) -> Option<&str> {
        self.content_type.as_deref()
    }

    /// The lowercased file extension derived from [`filename`] (no
    /// leading dot), if any.
    ///
    /// A registry can pass this to format resolution, e.g.
    /// `data.extension()` then `registry.by_extension(..)`.
    ///
    /// [`filename`]: Self::filename
    #[must_use]
    pub fn extension(&self) -> Option<String> {
        self.filename.as_deref().and_then(|name| {
            Path::new(name)
                .extension()
                .and_then(|e| e.to_str())
                .map(str::to_ascii_lowercase)
        })
    }

    /// Whether the content is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// The content as a byte slice.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.data
    }

    /// Clone the content into a `Bytes`.
    #[must_use]
    pub fn to_bytes(&self) -> Bytes {
        self.data.clone()
    }

    /// Consume into the underlying `Bytes`.
    #[must_use]
    pub fn into_bytes(self) -> Bytes {
        self.data
    }
}

impl From<&str> for ContentData {
    fn from(s: &str) -> Self {
        Self::from_text(s)
    }
}

impl From<String> for ContentData {
    fn from(s: String) -> Self {
        Self::from_text(s)
    }
}

impl From<&[u8]> for ContentData {
    fn from(bytes: &[u8]) -> Self {
        Self::new(Bytes::copy_from_slice(bytes))
    }
}

impl From<Vec<u8>> for ContentData {
    fn from(vec: Vec<u8>) -> Self {
        Self::new(Bytes::from(vec))
    }
}

impl From<Bytes> for ContentData {
    fn from(bytes: Bytes) -> Self {
        Self::new(bytes)
    }
}

impl fmt::Display for ContentData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<{} bytes>", self.data.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_defaults_to_none() {
        let c = ContentData::from_text("x");
        assert_eq!(c.filename(), None);
        assert_eq!(c.content_type(), None);
        assert_eq!(c.extension(), None);
    }

    #[test]
    fn decode_reads_bytes_as_text() {
        let c = ContentData::from_text("héllo");
        assert_eq!(c.decode().unwrap(), "héllo");
        // Invalid UTF-8 fails to decode.
        let bad = ContentData::new(Bytes::from_static(&[0xff, 0xfe]));
        assert!(bad.decode().is_err());
    }

    #[test]
    fn metadata_builders_set_fields() {
        let c = ContentData::from_text("x")
            .with_filename("Report.TXT")
            .with_content_type("text/plain");
        assert_eq!(c.filename(), Some("Report.TXT"));
        assert_eq!(c.content_type(), Some("text/plain"));
        // Extension is lowercased and dot-stripped.
        assert_eq!(c.extension().as_deref(), Some("txt"));
    }

    #[test]
    fn extension_handles_dotless_and_multidot_names() {
        assert_eq!(ContentData::new(Bytes::new()).extension(), None);
        assert_eq!(
            ContentData::from_text("x")
                .with_filename("noext")
                .extension(),
            None,
        );
        assert_eq!(
            ContentData::from_text("x")
                .with_filename("archive.tar.gz")
                .extension()
                .as_deref(),
            Some("gz"),
        );
    }
}
