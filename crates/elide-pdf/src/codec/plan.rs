//! [`Plan`]: everything an encode writes into a decoded document,
//! applied in one pass over one copy of it and saved once.

use elide_core::Result;

use crate::content::attachment::{AttachmentId, AttachmentRedaction, AttachmentReplacement};
use crate::content::image::{ImageId, ImageRedaction, ImageReplacement};
#[cfg(feature = "render")]
use crate::content::page::{PageRedaction, Repaint, ScannedPage};
use crate::content::text::{Detection, TextBlock, TextRedaction};
use crate::pdf::{Edit, Pdf};

/// Everything an encode writes into a document.
#[derive(Debug, Default)]
pub(super) struct Plan {
    /// The detected text spans whose glyphs are deleted.
    pub(super) text: Vec<Detection>,
    /// Embedded images replaced with their redacted bytes.
    pub(super) images: Vec<ImageReplacement>,
    /// Embedded images no codec could read, removed.
    pub(super) removed_images: Vec<ImageId>,
    /// Pages redacted from their render, each replaced by it with the
    /// regions the image pipeline redacted painted over.
    #[cfg(feature = "render")]
    pub(super) scanned: Vec<ScannedPage>,
    /// The scale the rendered pages were rendered at.
    #[cfg(feature = "render")]
    pub(super) scan_scale: f32,
    /// Attachments replaced with their redacted bytes.
    pub(super) attachments: Vec<AttachmentReplacement>,
    /// Attachments dropped from the document.
    pub(super) removed_attachments: Vec<AttachmentId>,
}

impl Plan {
    /// Whether the redaction changes nothing.
    pub(super) fn is_empty(&self) -> bool {
        #[cfg(feature = "render")]
        let scanned = self.scanned.is_empty();
        #[cfg(not(feature = "render"))]
        let scanned = true;
        self.text.is_empty()
            && self.images.is_empty()
            && self.removed_images.is_empty()
            && scanned
            && self.attachments.is_empty()
            && self.removed_attachments.is_empty()
    }

    /// `pdf` with the redaction applied: the glyphs of the detected text
    /// deleted (located through `blocks`, its [`Text`](crate::content::text::Text)),
    /// the redacted images and rendered pages written back, the unreadable
    /// images removed, and the attachments replaced or dropped.
    ///
    /// # Errors
    ///
    /// As each redaction:
    /// [`ErrorKind::Redaction`](elide_core::ErrorKind::Redaction) if a
    /// replacement or removal names nothing in the document, or a detected
    /// glyph cannot be found;
    /// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if a
    /// content stream cannot be rewritten or the document cannot be saved.
    pub(super) fn apply(&self, pdf: &Pdf, blocks: &[TextBlock]) -> Result<Pdf> {
        let mut edit = pdf.edit();
        edit.redact(TextRedaction {
            source: pdf,
            blocks,
            detections: &self.text,
        })?;
        self.redact_images(&mut edit)?;
        #[cfg(feature = "render")]
        if !self.scanned.is_empty() {
            let pages = if self.redraws_images() {
                edit.clone().finish()?.read(Repaint {
                    scanned: &self.scanned,
                    scale: self.scan_scale,
                })?
            } else {
                self.scanned
                    .iter()
                    .map(ScannedPage::replacement)
                    .collect::<Result<_>>()?
            };
            edit.redact(PageRedaction { pages: &pages })?;
        }
        self.redact_attachments(&mut edit)?;
        edit.finish()
    }

    /// Whether the redaction changes what an embedded image draws, so a page
    /// rendered before it no longer shows what the document draws.
    #[cfg(feature = "render")]
    fn redraws_images(&self) -> bool {
        !self.images.is_empty() || !self.removed_images.is_empty()
    }

    /// Write the image redactions into `edit`.
    fn redact_images(&self, edit: &mut Edit) -> Result<()> {
        edit.redact(ImageRedaction {
            replaced: &self.images,
            removed: &self.removed_images,
        })
    }

    /// Write the attachment redactions into `edit`.
    fn redact_attachments(&self, edit: &mut Edit) -> Result<()> {
        edit.redact(AttachmentRedaction {
            replaced: &self.attachments,
            removed: &self.removed_attachments,
        })
    }
}
