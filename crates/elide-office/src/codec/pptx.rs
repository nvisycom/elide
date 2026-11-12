//! PPTX codec: binds the PresentationML engine format to the shared
//! [`ooxml`](super::ooxml) codec adapter.
//!
//! Everything but the format identity lives in [`super::ooxml`]: the document is
//! a body [`ExtractStream`](elide_codec::extract::ExtractStream) over the
//! element-text blocks [`OoxmlPackage::extract`](crate::ooxml::OoxmlPackage::extract)
//! recovers plus its embedding / document-property blobs, re-packed via the
//! shared `OoxmlDocument`.

use elide_codec::{Format, FormatId};

use super::ooxml::{OoxmlCodec, OoxmlLoader};
use crate::pptx::SlideFormat;

/// The PPTX codec seam: PresentationML over the shared OOXML adapter.
#[derive(Debug)]
pub(crate) struct PptxCodec;

impl OoxmlCodec for PptxCodec {
    type Format = SlideFormat;

    const FORMAT_ID: FormatId = FormatId::new("elide.document.pptx");
    const LABEL: &'static str = "pptx";
}

/// Stable [`FormatId`] for the PPTX codec.
pub const FORMAT_ID: FormatId = PptxCodec::FORMAT_ID;

/// The PPTX [`Format`]: PowerPoint presentations, resolved by `.pptx`.
pub fn format() -> Format {
    Format::with_document_loader(FORMAT_ID, OoxmlLoader::<PptxCodec>::new())
        .with_extensions(["pptx"])
        .with_content_types([
            "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        ])
}
