//! The PDF engine: an opened document ([`Pdf`], over its [`Store`]) and what
//! the crate does with one.
//!
//! - [`rebuild`] writes it afresh with only what the codec understands, and
//!   checks an encoded one is such a rebuild;
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
pub(crate) mod name_tree;
pub(crate) mod object;
pub(crate) mod rebuild;
pub(crate) mod scope;
mod store;

use std::collections::HashSet;
#[cfg(feature = "render")]
use std::collections::{BTreeMap, BTreeSet};

use bytes::Bytes;
use elide_codec::StrippedContent;
use elide_core::Result;
use lopdf::ObjectId;

use self::attachment::{Attachment, AttachmentId, AttachmentReplacement};
use self::image::{ImageId, ImageReplacement, SurfacedImage};
pub(crate) use self::store::Store;
#[cfg(feature = "render")]
use crate::render::{self, PageObservation, PageReplacement, ScannedPage};
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

    /// The document rebuilt with only what the codec understands, with the
    /// record of what the source held that it did not copy (see [`rebuild`]).
    ///
    /// # Errors
    ///
    /// As [`rebuild::rebuild`], and
    /// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if
    /// the rebuilt document cannot be saved.
    pub(crate) fn rebuild(&self) -> Result<(Pdf, StrippedContent)> {
        let rebuilt = rebuild::rebuild(self.0.doc())?;
        Ok((Pdf(Store::saved(rebuilt.document)?), rebuilt.stripped))
    }

    /// The document's bytes.
    pub(crate) fn bytes(&self) -> Bytes {
        self.0.bytes()
    }

    /// Check that the document is a rebuild: what the codec runs on an encoded
    /// PDF before returning it.
    ///
    /// # Errors
    ///
    /// As [`rebuild::check`].
    pub(crate) fn check_rebuilt(&self) -> Result<()> {
        rebuild::check(self.0.doc())
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
    /// redacted images and pages written back, the unreadable images removed,
    /// and the attachments replaced or dropped.
    ///
    /// # Errors
    ///
    /// As each step: [`ErrorKind::Redaction`](elide_core::ErrorKind::Redaction) if a
    /// replacement or removal names nothing in the document, or a detected
    /// glyph cannot be found;
    /// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if a
    /// content stream cannot be rewritten or the document cannot be saved.
    pub(crate) fn apply(&self, blocks: &[TextBlock], redaction: &Redaction) -> Result<Pdf> {
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
            if !redaction.scanned.is_empty() {
                let redraws = redaction.redraws_images();
                let staged;
                let drawn = if redraws {
                    staged = Pdf(Store::saved(doc.clone())?);
                    &staged
                } else {
                    self
                };
                let pages = drawn.scanned(&redaction.scanned, redaction.scan_scale, redraws)?;
                render::reflatten(&mut doc, &pages)?;
            }
            render::reflatten(&mut doc, &redaction.pages)?;
        }
        attachment::replace(
            &mut doc,
            &redaction.attachments,
            &redaction.removed_attachments,
        )?;
        Store::saved(doc).map(Pdf)
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
    /// scanned page by its render with its redacted regions painted over,
    /// every other page rendered at `scale` with the glyphs `detections`
    /// cover in `pages` painted over. The embedded images are redacted first,
    /// so the render shows them redacted, and the attachments are replaced or
    /// dropped.
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
    ) -> Result<Pdf> {
        let redraws = redaction.redraws_images();
        let images = Redaction {
            images: redaction.images,
            removed_images: redaction.removed_images,
            ..Redaction::default()
        };
        let redacted;
        let pdf = if redraws {
            redacted = self.apply(&[], &images)?;
            &redacted
        } else {
            self
        };

        let scanned: BTreeSet<u32> = redaction.scanned.iter().map(|page| page.number).collect();
        let rendered: BTreeSet<u32> = pages
            .iter()
            .map(|page| page.page)
            .filter(|number| !scanned.contains(number))
            .collect();
        let boxes = render::glyph_boxes(pages, detections)?;
        let mut flat = render::flatten(&pdf.0, rendered, scale, &boxes)?;
        flat.extend(pdf.scanned(&redaction.scanned, scale, redraws)?);
        let pages = Redaction {
            pages: flat,
            attachments: redaction.attachments,
            removed_attachments: redaction.removed_attachments,
            ..Redaction::default()
        };
        pdf.apply(&[], &pages)
    }

    /// The images that replace the redacted `scanned` pages, rendered at
    /// `scale`. When `redrawn`, this document draws embedded images other than
    /// the render the pipeline was given, so each page's redacted regions are
    /// painted over a fresh render of it: its redacted images show, and every
    /// redacted region stays covered. Otherwise each page is its render as
    /// redacted.
    ///
    /// # Errors
    ///
    /// As [`render::repaint`] and [`ScannedPage::replacement`].
    #[cfg(feature = "render")]
    fn scanned(
        &self,
        scanned: &[ScannedPage],
        scale: f32,
        redrawn: bool,
    ) -> Result<Vec<PageReplacement>> {
        if redrawn {
            return render::repaint(&self.0, scanned, scale);
        }
        scanned.iter().map(ScannedPage::replacement).collect()
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
    /// Scanned pages the image pipeline redacted, each replaced by its render
    /// with the redacted regions painted over.
    #[cfg(feature = "render")]
    pub(crate) scanned: Vec<ScannedPage>,
    /// The scale the scanned pages were rendered at.
    #[cfg(feature = "render")]
    pub(crate) scan_scale: f32,
    /// Attachments replaced with their redacted bytes.
    pub(crate) attachments: Vec<AttachmentReplacement>,
    /// Attachments dropped from the document.
    pub(crate) removed_attachments: Vec<AttachmentId>,
}

impl Redaction {
    /// Whether the redaction changes what an embedded image draws, so a page
    /// rendered before it no longer shows what the document draws.
    #[cfg(feature = "render")]
    fn redraws_images(&self) -> bool {
        !self.images.is_empty() || !self.removed_images.is_empty()
    }

    /// Whether the redaction changes nothing.
    pub(crate) fn is_empty(&self) -> bool {
        #[cfg(feature = "render")]
        let pages = self.pages.is_empty() && self.scanned.is_empty();
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
    use crate::fixtures::with_pages;

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
        with_pages(
            &mut doc,
            [dictionary! {
                "Contents" => content,
                "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
                "MediaBox" => vec![0.into(), 0.into(), 100.into(), 100.into()],
            }],
        );
        (doc, content)
    }

    fn contains(haystack: &[u8], needle: &[u8]) -> bool {
        haystack.windows(needle.len()).any(|w| w == needle)
    }

    /// An incremental update leaves the superseded content in the file, outside
    /// the object graph; the rebuilt document never carries it.
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

        let (rebuilt, _) = Pdf::open(&updated).unwrap().rebuild().unwrap();
        assert!(!contains(&rebuilt.bytes(), b"Jane Roe"));
        assert!(contains(&rebuilt.bytes(), b"public"));
    }

    /// Every stream of the PDF `bytes`, decoded, as text.
    fn streams_of(bytes: &[u8]) -> String {
        let doc = Document::load_mem(bytes).unwrap();
        doc.objects
            .values()
            .filter_map(|object| object.as_stream().ok())
            .map(|stream| {
                String::from_utf8_lossy(
                    &stream
                        .decompressed_content()
                        .unwrap_or(stream.content.clone()),
                )
                .into_owned()
            })
            .collect()
    }

    /// Text a tiling pattern draws is detected and deleted like the page's:
    /// the pattern's one stream draws every tile.
    #[test]
    fn text_in_a_tiling_pattern_is_found_and_deleted() {
        tiling_text(Object::Integer(1), 1);
    }

    /// A tiling pattern whose `/PatternType` is the real `1.0` is still a
    /// tiling pattern, and its text is still found.
    #[test]
    fn a_real_pattern_type_is_still_a_tiling_pattern() {
        tiling_text(Object::Real(1.0), 1);
    }

    /// A pattern selected before every fill, more times than the page's walk
    /// budget, is walked once: its text reads once and the page is not
    /// refused.
    #[test]
    fn a_pattern_selected_many_times_is_walked_once() {
        tiling_text(Object::Integer(1), 5_000);
    }

    /// Find and delete the text a tiling pattern of `/PatternType` `kind`
    /// draws, the page selecting it `selections` times.
    fn tiling_text(kind: Object, selections: usize) {
        let mut doc = Document::with_version("1.5");
        let font = doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        });
        let cell = Content {
            operations: vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec!["F1".into(), 12.into()]),
                Operation::new("Tj", vec![Object::string_literal("Jane Roe")]),
                Operation::new("ET", vec![]),
            ],
        };
        let pattern = doc.add_object(Stream::new(
            dictionary! {
                "Type" => "Pattern", "PatternType" => kind, "PaintType" => 1, "TilingType" => 1,
                "BBox" => vec![0.into(), 0.into(), 50.into(), 20.into()],
                "XStep" => 50, "YStep" => 20,
                "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
            },
            cell.encode().unwrap(),
        ));
        let content = doc.add_object(Stream::new(
            dictionary! {},
            b"/Pattern cs /P0 scn 0 0 100 100 re f\n".repeat(selections),
        ));
        with_pages(
            &mut doc,
            [dictionary! {
                "Contents" => content,
                "Resources" => dictionary! { "Pattern" => dictionary! { "P0" => pattern } },
                "MediaBox" => vec![0.into(), 0.into(), 100.into(), 100.into()],
            }],
        );
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).unwrap();

        let (pdf, _) = Pdf::open(&bytes).unwrap().rebuild().unwrap();
        let blocks = pdf.text().unwrap();
        assert_eq!(blocks[0].text, "Jane Roe");
        let redaction = Redaction {
            text: vec![Detection::new(1, 0, 8)],
            ..Redaction::default()
        };
        let out = pdf.apply(&blocks, &redaction).unwrap().bytes();
        assert!(!streams_of(&out).contains("Jane Roe"));
        Pdf::open(&out).unwrap().check_rebuilt().unwrap();
    }
}
