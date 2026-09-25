//! [`FormatRegistry`]: resolves an extension or content type to a
//! registered [`Format`] and decodes content through its loader.
//!
//! Downstream crates register their own formats with
//! [`FormatRegistry::add_format`]; there is no central enum to extend.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;

use elide_codec::content::ContentData;
use elide_codec::{Document, Format, FormatId};
use elide_core::{Error, ErrorKind, Result};

/// Lowercase `s` for ASCII case-insensitive extension / content-type keys,
/// **borrowing** when it is already lowercase (the conventional case) so no
/// allocation happens. Only a key that actually carries an uppercase ASCII
/// letter is allocated. Used for both map keys and lookup queries, so a
/// case-insensitive hit never allocates on either side unless the input is
/// mixed-case.
fn ascii_lower_cow(s: &str) -> Cow<'_, str> {
    if s.bytes().any(|b| b.is_ascii_uppercase()) {
        Cow::Owned(s.to_ascii_lowercase())
    } else {
        Cow::Borrowed(s)
    }
}

/// Lowercase a `Format`'s extension / content-type into a lookup key while
/// **preserving its `'static` borrow** when it is already lowercase: a builtin
/// format's `Cow::Borrowed(&'static str)` becomes a borrowed key with no
/// allocation, and only a mixed-case (or already-owned mixed-case) value
/// allocates. This is the store-side counterpart to [`ascii_lower_cow`].
fn ascii_lower_key(s: Cow<'static, str>) -> Cow<'static, str> {
    if s.bytes().any(|b| b.is_ascii_uppercase()) {
        Cow::Owned(s.to_ascii_lowercase())
    } else {
        s
    }
}

/// Owns the registered [`Format`]s and resolves content to one of them.
///
/// Resolves by file extension, MIME content type, or [`FormatId`], then
/// decodes through the matched format's loader.
///
/// A registry is a cheap-to-`Clone` handle over immutable shared state: the
/// registered formats and their lookup maps live behind an [`Arc`], so cloning
/// is a single atomic bump rather than re-allocating the (dozens of) map keys.
/// A registry is built up once, [`with_builtin`] / [`with_format`], then
/// shared; the mutating builders use copy-on-write, so they only ever pay for a
/// deep clone if the registry is mutated *after* it has been shared, which the
/// build-then-share flow never does.
///
/// [`with_builtin`]: Self::with_builtin
/// [`with_format`]: Self::with_format
#[derive(Debug, Default, Clone)]
pub struct FormatRegistry(Arc<FormatRegistryInner>);

/// The shared, immutable body of a [`FormatRegistry`]. Held behind an [`Arc`]
/// so the public handle clones cheaply; all resolution logic lives here.
///
/// `Clone` exists only for [`Arc::make_mut`]'s copy-on-write in the mutating
/// builders, a live registry is never actually deep-cloned in the
/// build-then-share flow.
#[derive(Debug, Default, Clone)]
struct FormatRegistryInner {
    formats: Vec<Format>,
    by_id: HashMap<FormatId, usize>,
    by_extension: HashMap<Cow<'static, str>, usize>,
    by_content_type: HashMap<Cow<'static, str>, usize>,
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
        // Leaf text-shaped formats: handlers live in the `elide-plain` engine.
        #[cfg(feature = "txt")]
        registry.add_format(elide_plain::txt_format());
        #[cfg(feature = "json")]
        registry.add_format(elide_plain::json_format());
        #[cfg(feature = "html")]
        registry.add_format(elide_plain::html_format());
        #[cfg(feature = "xml")]
        registry.add_format(elide_plain::xml_format());
        #[cfg(feature = "csv")]
        registry.add_format(elide_plain::csv_format());
        // Audio formats: handlers live in the `elide-audio` engine.
        #[cfg(feature = "wav")]
        registry.add_format(elide_audio::codec::wav_format());
        #[cfg(feature = "mp3")]
        registry.add_format(elide_audio::codec::mp3_format());
        // Image formats: handlers live in the `elide-image` engine.
        #[cfg(feature = "png")]
        registry.add_format(elide_image::codec::png_format());
        #[cfg(feature = "jpeg")]
        registry.add_format(elide_image::codec::jpeg_format());
        #[cfg(feature = "tiff")]
        registry.add_format(elide_image::codec::tiff_format());
        #[cfg(any(feature = "png", feature = "jpeg", feature = "tiff"))]
        registry.add_format(elide_image::codec::exif_format());
        // OOXML formats: handlers live in the `elide-office` engine.
        #[cfg(feature = "xlsx")]
        registry.add_format(elide_office::codec::xlsx_format());
        #[cfg(feature = "docx")]
        registry.add_format(elide_office::codec::docx_format());
        #[cfg(feature = "internal_office")]
        registry.add_format(elide_office::codec::docprops_format());
        #[cfg(feature = "pptx")]
        registry.add_format(elide_office::codec::pptx_format());
        // PDF: handler lives in the `elide-pdf` engine.
        #[cfg(feature = "pdf")]
        registry.add_format(elide_pdf::codec::pdf_format());
        registry
    }

    /// Register a [`Format`] and return `self` for chained builder
    /// calls.
    ///
    /// # Panics
    ///
    /// Panics if the format's id is already registered, registering a new
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
        let inner = Arc::make_mut(&mut self.0);
        assert!(
            !inner.by_id.contains_key(format.id()),
            "format id already registered: {} (use replace_format to override)",
            format.id()
        );
        inner.insert_format(format);
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
    ///     .with_replaced_format(elide_pdf::codec::pdf_format_with(RasterMode::always()))
    ///     .with_replaced_format(elide_plain::html_format_with(ScanText, Skip));
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
        Arc::make_mut(&mut self.0).insert_format(format);
        self
    }

    /// Look up a registered format by id.
    pub fn by_id(&self, id: &FormatId) -> Option<&Format> {
        self.0.by_id.get(id).map(|&i| &self.0.formats[i])
    }

    /// Look up a registered format by file extension (case-insensitive,
    /// no leading dot).
    pub fn by_extension(&self, ext: &str) -> Option<&Format> {
        self.0
            .by_extension
            .get(ascii_lower_cow(ext).as_ref())
            .map(|&i| &self.0.formats[i])
    }

    /// Look up a registered format by MIME content type
    /// (case-insensitive).
    pub fn by_content_type(&self, mime: &str) -> Option<&Format> {
        self.0
            .by_content_type
            .get(ascii_lower_cow(mime).as_ref())
            .map(|&i| &self.0.formats[i])
    }

    /// Iterate over every registered format in registration order.
    pub fn iter(&self) -> impl Iterator<Item = &Format> {
        self.0.formats.iter()
    }

    /// Decode raw content using the format resolved from the extension
    /// hint. Accepts anything convertible into [`ContentData`]: `&str`,
    /// `&[u8]`, `Vec<u8>`, `Bytes`, `String`.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::CapabilityUnavailable`] when no format is
    /// registered for `extension`; otherwise propagates the loader's decode
    /// error.
    pub async fn decode(
        &self,
        content: impl Into<ContentData>,
        extension: &str,
    ) -> Result<Document> {
        let format = self.by_extension(extension).ok_or_else(|| {
            Error::new(
                ErrorKind::CapabilityUnavailable,
                format!("no codec registered for extension `{extension}`"),
            )
        })?;
        format.decode(content.into()).await
    }

    /// Decode [`ContentData`], resolving the format from the metadata it
    /// carries: its [`extension`] first, then its declared [`content_type`].
    ///
    /// With the `sniff` feature, a last resort infers the format from the
    /// leading bytes when the content asserts neither — so a caller who holds
    /// only raw bytes can still decode a binary format (the text-shaped formats
    /// carry no magic bytes). A caller-asserted extension or content type always
    /// wins over a sniff.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::CapabilityUnavailable`] when the format cannot be
    /// resolved from the content's extension, content type, or (with `sniff`)
    /// its bytes; otherwise propagates the loader's decode error.
    ///
    /// [`extension`]: ContentData::extension
    /// [`content_type`]: ContentData::content_type
    pub async fn decode_content(&self, content: ContentData) -> Result<Document> {
        let by_ext = content
            .extension()
            .and_then(|ext| self.by_extension(&ext))
            .map(|f| f.id().clone());
        let format_id = by_ext.or_else(|| {
            content
                .content_type()
                .and_then(|ct| self.by_content_type(ct))
                .map(|f| f.id().clone())
        });
        // Sniff the bytes only when the caller asserted nothing at all. An
        // extension or content type that is present but unregistered is still a
        // claim about the format, so honor the caller's intent by failing rather
        // than second-guessing it — the sniff is for the "no hints" case.
        let format_id = format_id.or_else(|| {
            (content.extension().is_none() && content.content_type().is_none())
                .then(|| self.sniff(content.as_bytes()).map(|f| f.id().clone()))
                .flatten()
        });
        let Some(format_id) = format_id else {
            return Err(Error::new(
                ErrorKind::CapabilityUnavailable,
                "content carries no resolvable filename extension or content type",
            ));
        };
        // `format_id` came from a lookup above, so this is present.
        let format = self.by_id(&format_id).expect("resolved format present");
        format.decode(content).await
    }

    /// The format inferred from the leading bytes' magic number, when the
    /// `sniff` feature is enabled and a registered format matches the inferred
    /// extension. Only the binary formats carry magic bytes; the text-shaped
    /// formats never match. Always `None` without the feature.
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
    /// Insert `format`, reusing the existing slot when its id is already
    /// registered (so the `usize` indices the lookup maps hold stay valid,
    /// removing the old entry would shift every later index), and re-point
    /// its extensions / content types to that slot.
    ///
    /// When replacing, the outgoing format's extension / content-type keys are
    /// first dropped, so a key the replacement no longer declares stops
    /// resolving to this slot rather than lingering as a stale mapping.
    fn insert_format(&mut self, format: Format) {
        let id = format.id().clone();
        let index = match self.by_id.get(&id) {
            Some(&existing) => {
                // Drop the outgoing format's keys that point here; the
                // replacement re-inserts whatever it still declares below.
                // Guard on the value so a key another format later claimed
                // (last-registration-wins) is left alone.
                self.by_extension.retain(|_, &mut i| i != existing);
                self.by_content_type.retain(|_, &mut i| i != existing);
                self.formats[existing] = format;
                existing
            }
            None => {
                let index = self.formats.len();
                self.formats.push(format);
                index
            }
        };
        let extensions = self.formats[index].extensions().to_vec();
        let content_types = self.formats[index].content_types().to_vec();
        for ext in extensions {
            self.by_extension.insert(ascii_lower_key(ext), index);
        }
        for ct in content_types {
            self.by_content_type.insert(ascii_lower_key(ct), index);
        }
        self.by_id.insert(id, index);
    }
}

#[cfg(all(test, feature = "txt"))]
mod tests {
    use elide_codec::Format;
    use elide_codec::test_util::MockLoader;
    use elide_plain::txt_format;

    use super::*;

    /// A format reusing the txt id but claiming a different extension, to
    /// stand in for a customized built-in. The registry bookkeeping under test
    /// never decodes through it, so any loader with the right id serves.
    fn txt_variant() -> Format {
        Format::with_document_loader(txt_format().id().clone(), MockLoader)
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
        let id = txt_format().id().clone();
        let mut reg = FormatRegistry::new();
        reg.add_format(txt_format());
        let before = reg.iter().count();

        reg.replace_format(txt_variant());

        // Same slot reused: no duplicate format.
        assert_eq!(reg.iter().count(), before);
        // The replacement's lookups now resolve to the (single) txt id.
        assert_eq!(
            reg.by_extension("variant").map(|f| f.id().clone()),
            Some(id.clone())
        );
        assert_eq!(
            reg.by_content_type("text/variant").map(|f| f.id().clone()),
            Some(id.clone())
        );
        // The original still resolves by id to the same single entry.
        assert!(reg.by_id(&id).is_some());
    }

    /// Replacing a format drops the outgoing format's extension /
    /// content-type keys that the replacement no longer declares, rather than
    /// leaving them mapped to the reused slot. `txt` declares `txt`/`log` +
    /// `text/plain`; the variant declares neither, so none of them may resolve
    /// after the swap.
    #[test]
    fn replace_format_drops_stale_keys() {
        let mut reg = FormatRegistry::new();
        reg.add_format(txt_format());
        // Present before the swap.
        assert!(reg.by_extension("txt").is_some());
        assert!(reg.by_extension("log").is_some());
        assert!(reg.by_content_type("text/plain").is_some());

        reg.replace_format(txt_variant());

        // The keys the replacement no longer declares stop resolving...
        assert!(reg.by_extension("txt").is_none());
        assert!(reg.by_extension("log").is_none());
        assert!(reg.by_content_type("text/plain").is_none());
        // ...and only the replacement's own keys resolve.
        assert!(reg.by_extension("variant").is_some());
        assert!(reg.by_content_type("text/variant").is_some());
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
