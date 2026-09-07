//! PPTX codec: binds the PresentationML engine format to the shared
//! [`ooxml`](super::ooxml) codec adapter.
//!
//! Everything but the format identity lives in [`super::ooxml`]: the handler is
//! an [`OoxmlHandler`] over the element-text blocks
//! [`Pptx::extract`](elide_office::pptx::Pptx::extract) recovers, re-packed via
//! the shared [`OoxmlEncoder`](super::ooxml::OoxmlEncoder).

use elide_core::modality::text::Text;
use elide_office::pptx::SlideFormat;

use super::PptxLoader;
use super::ooxml::{OoxmlCodec, OoxmlHandler};
use crate::{Format, FormatId};

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

/// Handler type for loaded PPTX content.
pub(crate) type PptxHandler = OoxmlHandler<PptxCodec>;

/// [`Format`] descriptor registered into [`FormatRegistry`].
///
/// [`FormatRegistry`]: crate::FormatRegistry
pub fn format() -> Format {
    Format::new::<Text, _>(FORMAT_ID.clone(), PptxLoader)
        .with_extensions(["pptx"])
        .with_content_types([
            "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        ])
}
