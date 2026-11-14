//! [`FormatRegistry`]: resolves an extension or content type to a registered
//! [`Format`] and decodes content through its loader.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;

use elide_codec::content::ContentData;
use elide_codec::{Document, Format, FormatId};
use elide_core::redaction::HiddenContentPolicy;
use elide_core::{Error, Result};

/// `s` lowercased for a case-insensitive key, borrowed when it already is.
fn ascii_lower(s: Cow<'_, str>) -> Cow<'_, str> {
    if s.bytes().any(|b| b.is_ascii_uppercase()) {
        Cow::Owned(s.to_ascii_lowercase())
    } else {
        s
    }
}

/// Owns the registered [`Format`]s and resolves content to one of them.
///
/// Resolves by file extension, MIME content type, or [`FormatId`], then
/// decodes through the matched format's loader under the caller's
/// [`HiddenContentPolicy`]. Downstream crates register their own formats with
/// [`with_format`]; there is no central enum to extend.
///
/// Cloning is cheap: the formats and lookup maps are shared behind an [`Arc`].
/// The builders copy them only if the registry was already cloned.
///
/// [`with_format`]: Self::with_format
#[derive(Debug, Default, Clone)]
pub struct FormatRegistry(Arc<FormatRegistryInner>);

/// The shared body of a [`FormatRegistry`]: the formats in registration order
/// and the lookup maps indexing into them.
#[derive(Debug, Default, Clone)]
struct FormatRegistryInner {
    formats: Vec<Format>,
    by_id: HashMap<FormatId, usize>,
    by_extension: HashMap<Cow<'static, str>, usize>,
    by_content_type: HashMap<Cow<'static, str>, usize>,
}

impl FormatRegistry {
    /// An empty registry. Add formats with [`with_format`], or start from
    /// [`with_builtin`].
    ///
    /// [`with_format`]: Self::with_format
    /// [`with_builtin`]: Self::with_builtin
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A registry of every built-in format the enabled features compile in.
    ///
    /// Formats added afterward take precedence on an extension or content type
    /// they share with a built-in (the last registration wins).
    #[must_use]
    pub fn with_builtin() -> Self {
        let mut registry = Self::new();
        #[cfg(feature = "txt")]
        registry.add_format(elide_plain::txt_format());
        #[cfg(feature = "json")]
        registry.add_format(elide_plain::json_format());
        #[cfg(feature = "csv")]
        registry.add_format(elide_plain::csv_format());
        #[cfg(feature = "html")]
        registry.add_format(elide_markup::html_format());
        #[cfg(feature = "xml")]
        registry.add_format(elide_markup::xml_format());
        #[cfg(feature = "wav")]
        registry.add_format(elide_audio::codec::wav_format());
        #[cfg(feature = "mp3")]
        registry.add_format(elide_audio::codec::mp3_format());
        #[cfg(feature = "png")]
        registry.add_format(elide_image::codec::png_format());
        #[cfg(feature = "jpeg")]
        registry.add_format(elide_image::codec::jpeg_format());
        #[cfg(feature = "tiff")]
        registry.add_format(elide_image::codec::tiff_format());
        #[cfg(feature = "xlsx")]
        registry.add_format(elide_office::codec::xlsx_format());
        #[cfg(feature = "docx")]
        registry.add_format(elide_office::codec::docx_format());
        #[cfg(feature = "pptx")]
        registry.add_format(elide_office::codec::pptx_format());
        #[cfg(feature = "pdf")]
        registry.add_format(elide_pdf::codec::pdf_format());
        registry
    }

    /// Register a [`Format`].
    ///
    /// An extension or content type another format already claims now
    /// resolves to this one (the last registration wins).
    ///
    /// # Panics
    ///
    /// If the format's id is already registered: a new format must not
    /// silently shadow an existing one. Override one deliberately with
    /// [`with_replaced_format`].
    ///
    /// [`with_replaced_format`]: Self::with_replaced_format
    #[must_use]
    pub fn with_format(mut self, format: Format) -> Self {
        self.add_format(format);
        self
    }

    /// In-place equivalent of [`with_format`].
    ///
    /// # Panics
    ///
    /// Same conditions as [`with_format`].
    ///
    /// [`with_format`]: Self::with_format
    pub fn add_format(&mut self, format: Format) -> &mut Self {
        assert!(
            !self.0.by_id.contains_key(&format.id()),
            "format id already registered: {} (use replace_format to override)",
            format.id()
        );
        Arc::make_mut(&mut self.0).insert_format(format);
        self
    }

    /// Register a [`Format`], replacing any with the same [`FormatId`], or
    /// adding it if there is none.
    ///
    /// The replaced format's extensions and content types stop resolving
    /// unless the replacement declares them too. Use it to customize a
    /// built-in while keeping the rest, such as swapping in
    /// `elide_pdf::codec::pdf_format_with` to flatten PDF pages:
    ///
    /// ```
    /// use elide_codec::Format;
    /// use elide_codec::mocks::{MOCK_FORMAT_ID, MockLoader, mock_format};
    /// use elide_format::FormatRegistry;
    ///
    /// let custom = Format::with_document_loader(MOCK_FORMAT_ID, MockLoader)
    ///     .with_extensions(["custom"]);
    /// let registry = FormatRegistry::new()
    ///     .with_format(mock_format())
    ///     .with_replaced_format(custom);
    ///
    /// assert_eq!(registry.iter().count(), 1);
    /// assert!(registry.by_extension("mock").is_none());
    /// assert_eq!(registry.by_extension("custom").map(Format::id), Some(MOCK_FORMAT_ID));
    /// ```
    #[must_use]
    pub fn with_replaced_format(mut self, format: Format) -> Self {
        self.replace_format(format);
        self
    }

    /// In-place equivalent of [`with_replaced_format`].
    ///
    /// [`with_replaced_format`]: Self::with_replaced_format
    pub fn replace_format(&mut self, format: Format) -> &mut Self {
        Arc::make_mut(&mut self.0).insert_format(format);
        self
    }

    /// The format registered under `id`.
    pub fn by_id(&self, id: &FormatId) -> Option<&Format> {
        self.0.by_id.get(id).map(|&i| &self.0.formats[i])
    }

    /// The format registered for a file extension (case-insensitive, no
    /// leading dot).
    pub fn by_extension(&self, ext: &str) -> Option<&Format> {
        self.0
            .by_extension
            .get(ascii_lower(Cow::Borrowed(ext)).as_ref())
            .map(|&i| &self.0.formats[i])
    }

    /// The format registered for a MIME content type (case-insensitive).
    pub fn by_content_type(&self, mime: &str) -> Option<&Format> {
        self.0
            .by_content_type
            .get(ascii_lower(Cow::Borrowed(mime)).as_ref())
            .map(|&i| &self.0.formats[i])
    }

    /// Every registered format, in registration order.
    pub fn iter(&self) -> impl Iterator<Item = &Format> {
        self.0.formats.iter()
    }

    /// Decode `content` with the format registered for `extension`, handling
    /// its hidden content under `policy`.
    ///
    /// # Errors
    ///
    /// [`CapabilityUnavailable`] if no format is registered for `extension`;
    /// otherwise the loader's decode error.
    ///
    /// [`CapabilityUnavailable`]: elide_core::ErrorKind::CapabilityUnavailable
    pub async fn decode(
        &self,
        content: impl Into<ContentData>,
        extension: &str,
        policy: HiddenContentPolicy,
    ) -> Result<Document> {
        let format = self.by_extension(extension).ok_or_else(|| {
            Error::capability_unavailable(format!(
                "no codec registered for extension `{extension}`"
            ))
        })?;
        format.decode(content.into(), policy).await
    }

    /// Decode `content`, resolving the format from what it carries: its
    /// [`extension`] first, then its [`content_type`]. Its hidden content is
    /// handled under `policy`.
    ///
    /// With the `sniff` feature, content that asserts neither resolves from
    /// its leading bytes, which only the binary formats carry. An extension or
    /// content type that is present but unregistered is still a claim about
    /// the format, so it fails rather than falling through to the sniff.
    ///
    /// # Errors
    ///
    /// [`CapabilityUnavailable`] if no format resolves; otherwise the loader's
    /// decode error.
    ///
    /// [`extension`]: ContentData::extension
    /// [`content_type`]: ContentData::content_type
    /// [`CapabilityUnavailable`]: elide_core::ErrorKind::CapabilityUnavailable
    pub async fn decode_content(
        &self,
        content: ContentData,
        policy: HiddenContentPolicy,
    ) -> Result<Document> {
        let extension = content.extension();
        let content_type = content.content_type();
        let format = match (&extension, content_type) {
            (None, None) => self.sniff(content.as_bytes()),
            _ => extension
                .as_deref()
                .and_then(|ext| self.by_extension(ext))
                .or_else(|| content_type.and_then(|ct| self.by_content_type(ct))),
        };
        let Some(format) = format else {
            return Err(Error::capability_unavailable(
                "content carries no resolvable filename extension or content type",
            ));
        };
        format.decode(content, policy).await
    }

    /// The format whose extension the leading bytes' magic number implies.
    #[cfg(feature = "sniff")]
    fn sniff(&self, bytes: &[u8]) -> Option<&Format> {
        let kind = infer::get(bytes)?;
        self.by_extension(kind.extension())
    }

    #[cfg(not(feature = "sniff"))]
    fn sniff(&self, _bytes: &[u8]) -> Option<&Format> {
        None
    }
}

impl FormatRegistryInner {
    /// Insert `format`, reusing its id's slot if registered so the indices
    /// the maps hold stay valid.
    ///
    /// On a replace, the outgoing format's keys still pointing at the slot are
    /// dropped first (one a later format claimed is left alone), so a key the
    /// replacement doesn't declare stops resolving.
    fn insert_format(&mut self, format: Format) {
        let id = format.id();
        let index = match self.by_id.get(&id) {
            Some(&existing) => {
                self.by_extension.retain(|_, &mut i| i != existing);
                self.by_content_type.retain(|_, &mut i| i != existing);
                self.formats[existing] = format;
                existing
            }
            None => {
                self.formats.push(format);
                self.formats.len() - 1
            }
        };
        let format = &self.formats[index];
        for ext in format.extensions() {
            self.by_extension.insert(ascii_lower(ext.clone()), index);
        }
        for ct in format.content_types() {
            self.by_content_type.insert(ascii_lower(ct.clone()), index);
        }
        self.by_id.insert(id, index);
    }
}

#[cfg(test)]
mod tests {
    use elide_codec::Format;
    use elide_codec::mocks::{MOCK_FORMAT_ID, MockLoader};

    use super::*;

    /// The mock format, with an extension and a content type to replace.
    fn original() -> Format {
        Format::with_document_loader(MOCK_FORMAT_ID, MockLoader)
            .with_extensions(["mock", "MOCK2"])
            .with_content_types(["application/x-mock"])
    }

    /// A format with the mock id declaring none of `original`'s keys.
    fn variant() -> Format {
        Format::with_document_loader(MOCK_FORMAT_ID, MockLoader)
            .with_extensions(["variant"])
            .with_content_types(["text/variant"])
    }

    #[test]
    #[should_panic(expected = "format id already registered")]
    fn add_format_panics_on_duplicate_id() {
        let mut reg = FormatRegistry::new();
        reg.add_format(original());
        reg.add_format(variant());
    }

    #[test]
    fn lookups_are_case_insensitive() {
        let reg = FormatRegistry::new().with_format(original());
        assert!(reg.by_extension("MOCK").is_some());
        assert!(reg.by_extension("mock2").is_some());
        assert!(reg.by_content_type("Application/X-Mock").is_some());
    }

    #[test]
    fn replace_format_swaps_in_place_and_drops_stale_keys() {
        let mut reg = FormatRegistry::new();
        reg.add_format(original());

        reg.replace_format(variant());

        assert_eq!(reg.iter().count(), 1);
        assert!(reg.by_extension("mock").is_none());
        assert!(reg.by_extension("mock2").is_none());
        assert!(reg.by_content_type("application/x-mock").is_none());
        let id = Some(MOCK_FORMAT_ID);
        assert_eq!(reg.by_extension("variant").map(Format::id), id);
        assert_eq!(reg.by_content_type("text/variant").map(Format::id), id);
        assert_eq!(reg.by_id(&MOCK_FORMAT_ID).map(Format::id), id);
    }

    #[test]
    fn replace_format_adds_when_id_absent() {
        let mut reg = FormatRegistry::new();
        reg.replace_format(original());
        assert_eq!(reg.iter().count(), 1);
        assert!(reg.by_extension("mock").is_some());
    }
}
