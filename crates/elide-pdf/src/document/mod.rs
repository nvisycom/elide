//! The PDF engine: an opened document ([`Pdf`], over its [`Store`]) and what
//! the crate does with one.
//!
//! - [`sanitize`] removes its hidden content, and checks an encoded one carries
//!   none;
//! - [`image`] and [`attachment`] surface, replace, and remove its embedded
//!   images and files;
//! - [`content`] rewrites its content streams, and [`object`] extends lopdf's
//!   object model with the reads the rest share.
//!
//! A redaction is applied in one pass over one copy of the document and saved
//! once ([`Redaction`]). Text extraction and glyph deletion live beside the
//! engine in [`text`], and page rendering in `render` behind the `render`
//! feature.

pub(crate) mod attachment;
pub(crate) mod content;
pub(crate) mod image;
pub(crate) mod object;
pub(crate) mod sanitize;
mod store;

use std::collections::HashSet;
#[cfg(feature = "render")]
use std::collections::{BTreeMap, BTreeSet};

#[cfg(feature = "render")]
use bytes::Bytes;
use elide_codec::StrippedContent;
use elide_core::Result;
use lopdf::ObjectId;

use self::attachment::{Attachment, AttachmentId, AttachmentReplacement};
use self::image::{ImageId, ImageReplacement, SurfacedImage};
use self::object::DocumentExt;
use self::sanitize::SanitizeDocumentExt;
pub(crate) use self::store::Store;
#[cfg(feature = "render")]
use crate::render::{self, PageObservation, PageReplacement};
use crate::text::{self, Detection, TextBlock};

/// An opened PDF document: parsed once, then reused across operations.
#[derive(Debug, Clone)]
pub(crate) struct Pdf(Store);

impl Pdf {
    /// Open a PDF from its bytes.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::ResourceLimit`](elide_core::ErrorKind::ResourceLimit) if the input
    /// exceeds [`MAX_DOCUMENT_BYTES`](Store::MAX_DOCUMENT_BYTES),
    /// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if the
    /// bytes are not a readable PDF, or
    /// [`ErrorKind::CapabilityUnavailable`](elide_core::ErrorKind::CapabilityUnavailable)
    /// if it is encrypted with a user password.
    pub(crate) fn open(document: &[u8]) -> Result<Self> {
        Store::open(document).map(Self)
    }

    /// The document with its hidden content removed, as new bytes with the
    /// record of what was removed: metadata, annotations other than internal
    /// links, actions and scripts, form values, the outline and structure
    /// tree, optional-content layers, thumbnails, and every object no longer
    /// reachable from the document.
    ///
    /// The document is always re-serialized, so what the source file carries
    /// outside its object graph (a prior revision's superseded objects, bytes
    /// after the end marker) never reaches the output.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if a
    /// content stream cannot be rewritten or the sanitized document cannot be
    /// saved.
    pub(crate) fn sanitize(&self) -> Result<(Vec<u8>, StrippedContent)> {
        let mut doc = self.0.clone_doc();
        let stripped = doc.sanitize()?;
        Ok((doc.save_pruned()?, stripped))
    }

    /// Check that the document carries no hidden content: what the codec runs
    /// on an encoded PDF before returning it.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Sanitization`](elide_core::ErrorKind::Sanitization) naming what
    /// survived.
    pub(crate) fn check_sanitized(&self) -> Result<()> {
        self.0.doc().check_sanitized()
    }

    /// Every page's text, with the map from each character back to the glyph
    /// that drew it and that glyph's box.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) or
    /// [`ErrorKind::ResourceLimit`](elide_core::ErrorKind::ResourceLimit) for a
    /// page whose content cannot be read, and
    /// [`ErrorKind::Redaction`](elide_core::ErrorKind::Redaction) for text that
    /// cannot be located: a page that cannot be read is never treated as one
    /// with nothing to redact.
    pub(crate) fn text(&self) -> Result<Vec<TextBlock>> {
        text::text_blocks(&self.0)
    }

    /// Every embedded image, as the image pipeline reads it.
    ///
    /// # Errors
    ///
    /// As [`image::surfaced`].
    pub(crate) fn images(&self) -> Result<Vec<SurfacedImage>> {
        image::surfaced(self.0.doc())
    }

    /// Every attached file, decoded, in tree order.
    ///
    /// # Errors
    ///
    /// As [`attachment::attachments`].
    pub(crate) fn attachments(&self) -> Result<Vec<Attachment>> {
        attachment::attachments(&self.0)
    }

    /// The document with `redaction` applied, every step on one copy of the
    /// document and saved once: the glyphs of the detected text deleted
    /// (located through `blocks`, this document's [`text`](Pdf::text)), the
    /// redacted images and pages written back, the unreadable ones removed or
    /// cleared, and the attachments replaced or dropped.
    ///
    /// # Errors
    ///
    /// As each step: [`ErrorKind::Redaction`](elide_core::ErrorKind::Redaction) if a
    /// replacement or removal names nothing in the document, or a detected
    /// glyph cannot be found;
    /// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if a
    /// content stream cannot be rewritten or the document cannot be saved.
    pub(crate) fn apply(&self, blocks: &[TextBlock], redaction: &Redaction) -> Result<Vec<u8>> {
        let mut doc = self.0.clone_doc();
        if !redaction.text.is_empty() {
            text::redact::delete(&self.0, blocks, &mut doc, &redaction.text)?;
        }
        image::replace(&mut doc, &redaction.images)?;
        if !redaction.removed_images.is_empty() {
            let removed: HashSet<ObjectId> = redaction
                .removed_images
                .iter()
                .map(|id| id.object())
                .collect();
            image::remove(&mut doc, &removed, false)?;
        }
        #[cfg(feature = "render")]
        {
            render::reflatten(&mut doc, &redaction.pages)?;
            render::clear(&mut doc, &redaction.cleared_pages)?;
        }
        attachment::replace(
            &mut doc,
            &redaction.attachments,
            &redaction.removed_attachments,
        )?;
        doc.save_pruned()
    }

    /// The 1-based pages `numbers` rendered at `scale` to PNG images, keyed by
    /// page number.
    ///
    /// # Errors
    ///
    /// As [`render::render_pages`].
    #[cfg(feature = "render")]
    pub(crate) fn render_pages(
        &self,
        numbers: BTreeSet<u32>,
        scale: f32,
    ) -> Result<BTreeMap<u32, Bytes>> {
        render::render_pages(&self.0, numbers, scale)
    }

    /// Every page's text as PDFium reads it, with each glyph's box.
    ///
    /// # Errors
    ///
    /// As [`render::observe`].
    #[cfg(feature = "render")]
    pub(crate) fn observe(&self) -> Result<Vec<PageObservation>> {
        render::observe(&self.0)
    }

    /// The document with every page replaced by a redacted image of it: a
    /// scanned page by its redacted blob in `redaction`, every other page
    /// rendered at `scale` with the glyphs `detections` cover in `pages`
    /// painted over. The embedded images are redacted first, so the render
    /// shows them redacted, and the attachments are replaced or dropped.
    ///
    /// # Errors
    ///
    /// As [`render::glyph_boxes`], [`render::flatten`], and
    /// [`apply`](Pdf::apply).
    #[cfg(feature = "render")]
    pub(crate) fn flatten(
        &self,
        pages: &[PageObservation],
        scale: f32,
        detections: &[Detection],
        redaction: Redaction,
    ) -> Result<Vec<u8>> {
        let images = Redaction {
            images: redaction.images,
            removed_images: redaction.removed_images,
            ..Redaction::default()
        };
        let redacted;
        let pdf = if images.is_empty() {
            self
        } else {
            redacted = Pdf::open(&self.apply(&[], &images)?)?;
            &redacted
        };

        let scanned: BTreeSet<u32> = redaction
            .pages
            .iter()
            .map(|page| page.number)
            .chain(redaction.cleared_pages.iter().copied())
            .collect();
        let rendered: BTreeSet<u32> = pages
            .iter()
            .map(|page| page.page)
            .filter(|number| !scanned.contains(number))
            .collect();
        let boxes = render::glyph_boxes(pages, detections)?;
        let mut flat = render::flatten(&pdf.0, rendered, scale, &boxes)?;
        flat.extend(redaction.pages);
        let pages = Redaction {
            pages: flat,
            cleared_pages: redaction.cleared_pages,
            attachments: redaction.attachments,
            removed_attachments: redaction.removed_attachments,
            ..Redaction::default()
        };
        pdf.apply(&[], &pages)
    }
}

/// Everything an encode writes into a document, applied by [`Pdf::apply`] in
/// one pass and saved once.
#[derive(Debug, Default)]
pub(crate) struct Redaction {
    /// The detected text spans whose glyphs are deleted.
    pub(crate) text: Vec<Detection>,
    /// Embedded images replaced with their redacted bytes.
    pub(crate) images: Vec<ImageReplacement>,
    /// Embedded images no codec could read, removed.
    pub(crate) removed_images: Vec<ImageId>,
    /// Pages replaced by a single redacted image.
    #[cfg(feature = "render")]
    pub(crate) pages: Vec<PageReplacement>,
    /// Scanned pages no codec could read, cleared.
    #[cfg(feature = "render")]
    pub(crate) cleared_pages: Vec<u32>,
    /// Attachments replaced with their redacted bytes.
    pub(crate) attachments: Vec<AttachmentReplacement>,
    /// Attachments dropped from the document.
    pub(crate) removed_attachments: Vec<AttachmentId>,
}

impl Redaction {
    /// Whether the redaction changes nothing.
    pub(crate) fn is_empty(&self) -> bool {
        #[cfg(feature = "render")]
        let pages = self.pages.is_empty() && self.cleared_pages.is_empty();
        #[cfg(not(feature = "render"))]
        let pages = true;
        self.text.is_empty()
            && self.images.is_empty()
            && self.removed_images.is_empty()
            && pages
            && self.attachments.is_empty()
            && self.removed_attachments.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use lopdf::content::{Content, Operation};
    use lopdf::{Document, IncrementalDocument, Object, Stream, dictionary};

    use super::*;

    /// A one-page document whose page shows `text`, and its content id.
    fn showing(text: &str) -> (Document, ObjectId) {
        let mut doc = Document::with_version("1.5");
        let font = doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        });
        let operations = vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 12.into()]),
            Operation::new("Tj", vec![Object::string_literal(text)]),
            Operation::new("ET", vec![]),
        ];
        let content = doc.add_object(Stream::new(
            dictionary! {},
            Content { operations }.encode().unwrap(),
        ));
        let pages = doc.new_object_id();
        let page = doc.add_object(dictionary! {
            "Type" => "Page", "Parent" => pages, "Contents" => content,
            "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
            "MediaBox" => vec![0.into(), 0.into(), 100.into(), 100.into()],
        });
        doc.objects.insert(
            pages,
            Object::Dictionary(dictionary! {
                "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1,
            }),
        );
        let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages });
        doc.trailer.set("Root", catalog);
        (doc, content)
    }

    fn contains(haystack: &[u8], needle: &[u8]) -> bool {
        haystack.windows(needle.len()).any(|w| w == needle)
    }

    /// An incremental update leaves the superseded content in the file, outside
    /// the object graph; the sanitized document never carries it.
    #[test]
    fn a_superseded_revision_never_ships() {
        let (mut doc, content) = showing("Jane Roe");
        let mut first = Vec::new();
        doc.save_to(&mut first).unwrap();

        let mut update =
            IncrementalDocument::create_from(first.clone(), Document::load_mem(&first).unwrap());
        update.opt_clone_object_to_new_document(content).unwrap();
        let stream = update
            .new_document
            .get_object_mut(content)
            .and_then(Object::as_stream_mut)
            .unwrap();
        stream.set_plain_content(b"BT /F1 12 Tf (public) Tj ET".to_vec());
        let mut updated = Vec::new();
        update.save_to(&mut updated).unwrap();
        assert!(
            contains(&updated, b"Jane Roe"),
            "the fixture keeps the old revision"
        );

        let (sanitized, _) = Pdf::open(&updated).unwrap().sanitize().unwrap();
        assert!(!contains(&sanitized, b"Jane Roe"));
        assert!(contains(&sanitized, b"public"));
    }
}
