//! [`FormatRegistry`]: resolves an extension or content type to a
//! registered [`Format`] and decodes content through its loader.
//!
//! Downstream crates register their own formats with
//! [`FormatRegistry::add_format`]; there is no central enum to extend.

use std::collections::HashMap;

use elide_core::{Error, ErrorKind, Result};

use super::document::UntypedDocumentHandle;
use super::{Format, FormatId};
use crate::content::ContentData;

/// Owns the registered [`Format`]s and resolves content to one of them.
///
/// Resolves by file extension, MIME content type, or [`FormatId`], then
/// decodes through the matched format's loader.
#[derive(Debug, Default)]
pub struct FormatRegistry {
    formats: Vec<Format>,
    by_id: HashMap<FormatId, usize>,
    by_extension: HashMap<String, usize>,
    by_content_type: HashMap<String, usize>,
}

impl FormatRegistry {
    /// Empty registry. Use [`with_format`] / [`add_format`] to add custom
    /// formats, or [`with_builtin`] to start from a pre-populated set of
    /// every built-in format the active feature set enables.
    ///
    /// [`with_format`]: Self::with_format
    /// [`add_format`]: Self::add_format
    /// [`with_builtin`]: Self::with_builtin
    pub fn new() -> Self {
        Self::default()
    }

    /// Pre-populated registry containing every built-in format the
    /// active feature set enables (TXT, JSON, Markdown, HTML, and so on).
    ///
    /// Add custom formats afterward with [`with_format`] (chainable) or
    /// [`add_format`] (in-place); they take precedence on extension /
    /// content-type collisions (last registration wins).
    ///
    /// [`with_format`]: Self::with_format
    /// [`add_format`]: Self::add_format
    pub fn with_builtin() -> Self {
        let mut registry = Self::new();
        #[cfg(feature = "txt")]
        registry.add_format(crate::handler::txt_format());
        #[cfg(feature = "json")]
        registry.add_format(crate::handler::json_format());
        #[cfg(feature = "html")]
        registry.add_format(crate::handler::html_format());
        #[cfg(feature = "xml")]
        registry.add_format(crate::handler::xml_format());
        #[cfg(feature = "png")]
        registry.add_format(crate::handler::png_format());
        #[cfg(feature = "jpeg")]
        registry.add_format(crate::handler::jpeg_format());
        #[cfg(feature = "tiff")]
        registry.add_format(crate::handler::tiff_format());
        #[cfg(feature = "wav")]
        registry.add_format(crate::handler::wav_format());
        #[cfg(feature = "mp3")]
        registry.add_format(crate::handler::mp3_format());
        #[cfg(feature = "csv")]
        registry.add_format(crate::handler::csv_format());
        #[cfg(feature = "xlsx")]
        registry.add_format(crate::handler::xlsx_format());
        #[cfg(feature = "docx")]
        registry.add_format(crate::handler::docx_format());
        #[cfg(feature = "pdf")]
        registry.add_format(crate::handler::pdf_format());
        #[cfg(feature = "rtf")]
        registry.add_format(crate::handler::rtf_format());
        registry
    }

    /// Register a [`Format`] and return `self` for chained builder
    /// calls.
    ///
    /// # Panics
    ///
    /// Panics if the format's id is already registered — registering a new
    /// format must not silently shadow an existing one. To deliberately
    /// override a built-in (e.g. swap in the OCR-enabled PDF format), use
    /// [`with_replaced_format`]. Extensions and content types that conflict
    /// with an existing format are overwritten (last registration wins);
    /// register custom formats *after* [`with_builtin`] for precedence.
    ///
    /// [`with_replaced_format`]: Self::with_replaced_format
    /// [`with_builtin`]: Self::with_builtin
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
            !self.by_id.contains_key(&format.id),
            "format id already registered: {} (use replace_format to override)",
            format.id
        );
        self.insert_format(format);
        self
    }

    /// Register a [`Format`], **replacing** any already registered under the
    /// same [`FormatId`], and return `self` for chained builder calls.
    ///
    /// This is the explicit override path: where [`with_format`] panics on a
    /// duplicate id, this swaps the existing format out in place. Use it to
    /// customize a built-in while keeping the rest:
    ///
    /// ```ignore
    /// let registry = FormatRegistry::with_builtin()
    ///     .with_replaced_format(handler::pdf_format_with(OcrMode::Force))
    ///     .with_replaced_format(handler::html_format_with(ScanText, Skip));
    /// ```
    ///
    /// Registering a format whose id is *not* present behaves like
    /// [`with_format`] (it is simply added).
    ///
    /// [`with_format`]: Self::with_format
    #[must_use]
    pub fn with_replaced_format(mut self, format: Format) -> Self {
        self.replace_format(format);
        self
    }

    /// In-place equivalent of [`with_replaced_format`].
    ///
    /// [`with_replaced_format`]: Self::with_replaced_format
    pub fn replace_format(&mut self, format: Format) -> &mut Self {
        self.insert_format(format);
        self
    }

    /// Insert `format`, reusing the existing slot when its id is already
    /// registered (so the `usize` indices the lookup maps hold stay valid —
    /// removing the old entry would shift every later index), and re-point
    /// its extensions / content types to that slot.
    fn insert_format(&mut self, format: Format) {
        let id = format.id.clone();
        let index = match self.by_id.get(&id) {
            Some(&existing) => {
                self.formats[existing] = format;
                existing
            }
            None => {
                let index = self.formats.len();
                self.formats.push(format);
                index
            }
        };
        let extensions = self.formats[index].extensions.clone();
        let content_types = self.formats[index].content_types.clone();
        for ext in &extensions {
            self.by_extension.insert(ext.to_ascii_lowercase(), index);
        }
        for ct in &content_types {
            self.by_content_type.insert(ct.to_ascii_lowercase(), index);
        }
        self.by_id.insert(id, index);
    }

    /// Look up a registered format by id.
    pub fn by_id(&self, id: &FormatId) -> Option<&Format> {
        self.by_id.get(id).map(|&i| &self.formats[i])
    }

    /// Look up a registered format by file extension (case-insensitive,
    /// no leading dot).
    pub fn by_extension(&self, ext: &str) -> Option<&Format> {
        self.by_extension
            .get(&ext.to_ascii_lowercase())
            .map(|&i| &self.formats[i])
    }

    /// Look up a registered format by MIME content type
    /// (case-insensitive).
    pub fn by_content_type(&self, mime: &str) -> Option<&Format> {
        self.by_content_type
            .get(&mime.to_ascii_lowercase())
            .map(|&i| &self.formats[i])
    }

    /// Iterate over every registered format in registration order.
    pub fn iter(&self) -> impl Iterator<Item = &Format> {
        self.formats.iter()
    }

    /// Decode raw content using the format resolved from the extension
    /// hint. Accepts anything convertible into [`ContentData`]: `&str`,
    /// `&[u8]`, `Vec<u8>`, `Bytes`, `String`.
    ///
    /// # Errors
    ///
    /// Returns a validation error when no format is registered for
    /// `extension`; otherwise propagates the loader's decode error.
    pub async fn decode(
        &self,
        content: impl Into<ContentData>,
        extension: &str,
    ) -> Result<UntypedDocumentHandle> {
        let format = self.by_extension(extension).ok_or_else(|| {
            Error::new(
                ErrorKind::Validation,
                format!("no codec registered for extension `{extension}`"),
            )
        })?;
        format.decode(content.into()).await
    }

    /// Decode [`ContentData`], resolving the format from the metadata it
    /// carries: its [`extension`] first, then its declared
    /// [`content_type`].
    ///
    /// # Errors
    ///
    /// Returns a validation error when the content carries neither a
    /// resolvable filename extension nor a known content type; otherwise
    /// propagates the loader's decode error.
    ///
    /// [`extension`]: ContentData::extension
    /// [`content_type`]: ContentData::content_type
    pub async fn decode_content(&self, content: ContentData) -> Result<UntypedDocumentHandle> {
        let by_ext = content
            .extension()
            .and_then(|ext| self.by_extension(&ext))
            .map(|f| f.id.clone());
        let format_id = by_ext.or_else(|| {
            content
                .content_type()
                .and_then(|ct| self.by_content_type(ct))
                .map(|f| f.id.clone())
        });
        let Some(format_id) = format_id else {
            return Err(Error::new(
                ErrorKind::Validation,
                "content carries no resolvable filename extension or content type",
            ));
        };
        // `format_id` came from a lookup above, so this is present.
        let format = self.by_id(&format_id).expect("resolved format present");
        format.decode(content).await
    }
}

#[cfg(all(test, feature = "txt"))]
mod tests {
    use elide_core::modality::text::Text;

    use super::*;
    use crate::Format;
    use crate::handler::text::TxtLoader;
    use crate::handler::txt_format;

    /// A format reusing the txt id but claiming a different extension, to
    /// stand in for a customized built-in.
    fn txt_variant() -> Format {
        Format::new::<Text, _>(txt_format().id.clone(), TxtLoader)
            .with_extensions(["variant"])
            .with_content_types(["text/variant"])
    }

    #[test]
    #[should_panic(expected = "format id already registered")]
    fn add_format_panics_on_duplicate_id() {
        let mut reg = FormatRegistry::new();
        reg.add_format(txt_format());
        reg.add_format(txt_variant()); // same id -> panic
    }

    #[test]
    fn replace_format_swaps_in_place() {
        let id = txt_format().id.clone();
        let mut reg = FormatRegistry::new();
        reg.add_format(txt_format());
        let before = reg.iter().count();

        reg.replace_format(txt_variant());

        // Same slot reused: no duplicate format.
        assert_eq!(reg.iter().count(), before);
        // The replacement's lookups now resolve to the (single) txt id.
        assert_eq!(reg.by_extension("variant").map(|f| f.id.clone()), Some(id.clone()));
        assert_eq!(
            reg.by_content_type("text/variant").map(|f| f.id.clone()),
            Some(id.clone())
        );
        // The original still resolves by id to the same single entry.
        assert!(reg.by_id(&id).is_some());
    }

    #[test]
    fn replace_format_adds_when_id_absent() {
        // With no prior registration, replace behaves like add.
        let mut reg = FormatRegistry::new();
        reg.replace_format(txt_format());
        assert_eq!(reg.iter().count(), 1);
        assert!(reg.by_extension("txt").is_some());
    }
}
