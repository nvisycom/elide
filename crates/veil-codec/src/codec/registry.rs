//! [`CodecRegistry`]: resolves an extension or content type to a
//! registered [`Format`] and decodes content through its loader.
//!
//! Downstream crates register their own formats with
//! [`CodecRegistry::add_format`] — there is no central enum to extend.

use std::collections::HashMap;

use veil_core::{Error, ErrorKind};

use super::{Format, FormatId};
use crate::content::ContentData;
use super::document::UntypedDocumentHandle;

/// Codec registry — owns the set of registered [`Format`]s and resolves
/// them by extension, content type, or id.
#[derive(Debug, Default)]
pub struct CodecRegistry {
    formats: Vec<Format>,
    by_id: HashMap<FormatId, usize>,
    by_extension: HashMap<String, usize>,
    by_content_type: HashMap<String, usize>,
}

impl CodecRegistry {
    /// Empty registry. Use [`with_format`](Self::with_format) /
    /// [`add_format`](Self::add_format) to add custom formats, or
    /// [`with_builtin`](Self::with_builtin) to start from a pre-
    /// populated set of every built-in format the active feature set
    /// enables.
    pub fn new() -> Self {
        Self::default()
    }

    /// Pre-populated registry containing every built-in format the
    /// active feature set enables (TXT, JSON, Markdown, HTML, …).
    ///
    /// Add custom formats afterward with
    /// [`with_format`](Self::with_format) (chainable) or
    /// [`add_format`](Self::add_format) (in-place); they take precedence
    /// on extension / content-type collisions (last registration wins).
    pub fn with_builtin() -> Self {
        let mut registry = Self::new();
        #[cfg(feature = "txt")]
        registry.add_format(crate::handler::text::txt_format());
        #[cfg(feature = "json")]
        registry.add_format(crate::handler::text::json_format());
        #[cfg(feature = "html")]
        registry.add_format(crate::handler::markup::html_format());
        registry
    }

    /// Register a [`Format`] and return `self` for chained builder
    /// calls.
    ///
    /// # Panics
    ///
    /// Panics if the format's id is already registered. Extensions and
    /// content types that conflict with an existing format are
    /// overwritten (last registration wins) — register custom formats
    /// *after* [`with_builtin`](Self::with_builtin) for precedence.
    #[must_use]
    pub fn with_format(mut self, format: Format) -> Self {
        self.add_format(format);
        self
    }

    /// In-place equivalent of [`with_format`](Self::with_format).
    ///
    /// # Panics
    ///
    /// Same conditions as [`with_format`](Self::with_format).
    pub fn add_format(&mut self, format: Format) -> &mut Self {
        assert!(
            !self.by_id.contains_key(&format.id),
            "format id already registered: {}",
            format.id
        );
        let index = self.formats.len();
        for ext in &format.extensions {
            self.by_extension.insert(ext.to_ascii_lowercase(), index);
        }
        for ct in &format.content_types {
            self.by_content_type.insert(ct.to_ascii_lowercase(), index);
        }
        self.by_id.insert(format.id.clone(), index);
        self.formats.push(format);
        self
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
    /// hint. Accepts anything convertible into [`ContentData`] — `&str`,
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
    ) -> Result<UntypedDocumentHandle, Error> {
        let format = self.by_extension(extension).ok_or_else(|| {
            Error::new(
                ErrorKind::Validation,
                format!("no codec registered for extension `{extension}`"),
            )
        })?;
        format.decode(content.into()).await
    }

    /// Decode [`ContentData`], resolving the format from the metadata it
    /// carries: its [`extension`](ContentData::extension) first, then its
    /// declared [`content_type`](ContentData::content_type).
    ///
    /// # Errors
    ///
    /// Returns a validation error when the content carries neither a
    /// resolvable filename extension nor a known content type; otherwise
    /// propagates the loader's decode error.
    pub async fn decode_content(
        &self,
        content: ContentData,
    ) -> Result<UntypedDocumentHandle, Error> {
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
