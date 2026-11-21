//! PPTX codec: binds the PresentationML engine format to the shared
//! [`ooxml`](super::ooxml) codec adapter.
//!
//! Everything but the format identity lives in [`super::ooxml`]: the document is
//! a body [`ExtractStream`](elide_codec::extract::ExtractStream) over the
//! element-text blocks [`OoxmlPackage::extract`](crate::ooxml::OoxmlPackage::extract)
//! recovers plus its embedding / document-property blobs, re-packed via the
//! shared `OoxmlDocument`.

use elide_codec::{Format, FormatId};

use super::CodecIdentity;
use super::ooxml::{self, OoxmlCodec};
use crate::pptx::SlideFormat;

/// The PPTX codec seam: PresentationML over the shared OOXML adapter.
#[derive(Debug)]
pub(crate) struct PptxCodec;

impl CodecIdentity for PptxCodec {
    const CONTENT_TYPE: &'static str =
        "application/vnd.openxmlformats-officedocument.presentationml.presentation";
    const EXTENSION: &'static str = "pptx";
    const FORMAT_ID: FormatId = FormatId::new("elide.document.pptx");
}

impl OoxmlCodec for PptxCodec {
    type Format = SlideFormat;
}

/// The PPTX [`Format`]: PowerPoint presentations, resolved by `.pptx`.
pub fn format() -> Format {
    ooxml::format::<PptxCodec>()
}
