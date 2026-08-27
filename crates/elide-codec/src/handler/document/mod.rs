//! Rich-document formats: DOCX, PDF, RTF.
//!
//! Some are *containers of parts across modalities*: a DOCX is a zip whose
//! body text lives in XML parts (`word/document.xml`, headers, footers)
//! and whose images live as separate files (`word/media/*`); a PDF holds
//! page text and image XObjects in its own object format. The container is
//! parsed, each text part is redacted (DOCX reuses the shared XML
//! [`extract`] engine), and the document is rebuilt with only those parts
//! changed — every other byte round-trips. Others, like RTF, are flat text
//! with no parts, handled as plain text handlers.
//!
//! Following Tika's recursive model, embedded media are not flattened
//! into the text stream; they are exposed as their own decodable handles
//! for the toolkit to drive through the image pipeline (lazy, opt-in —
//! see the media accessor).
//!
//! [`extract`]: crate::handler::extract

#[cfg(feature = "docx")]
mod docx_handler;
#[cfg(feature = "docx")]
mod docx_loader;
#[cfg(feature = "pdf")]
mod pdf_handler;
#[cfg(feature = "pdf")]
mod pdf_loader;
#[cfg(feature = "pptx")]
mod pptx_handler;
#[cfg(feature = "pptx")]
mod pptx_loader;
#[cfg(feature = "rtf")]
mod rtf_handler;
#[cfg(feature = "rtf")]
mod rtf_loader;

#[cfg(feature = "pdf")]
pub use elide_core::primitive::RasterMode;

#[cfg(feature = "docx")]
pub use self::docx_handler::format as docx_format;
#[cfg(feature = "docx")]
pub(crate) use self::docx_loader::DocxLoader;
#[cfg(feature = "pdf")]
pub use self::pdf_handler::format as pdf_format;
#[cfg(feature = "pdf-render")]
pub use self::pdf_handler::format_with as pdf_format_with;
#[cfg(feature = "pdf")]
pub(crate) use self::pdf_loader::PdfLoader;
#[cfg(feature = "pptx")]
pub use self::pptx_handler::format as pptx_format;
#[cfg(feature = "pptx")]
pub(crate) use self::pptx_loader::PptxLoader;
#[cfg(feature = "rtf")]
pub use self::rtf_handler::format as rtf_format;
#[cfg(feature = "rtf")]
pub(crate) use self::rtf_loader::RtfLoader;

// Shared OPC part-addressing helpers for the container formats (DOCX, PPTX)
// whose blocks are addressed by `(part, span, OffsetMap)`. Keeping the
// decoded↔raw mapping in one place stops the two encoders' `source_span` /
// `locate_source` impls from drifting.
#[cfg(any(feature = "docx", feature = "pptx"))]
mod opc_source {
    use std::ops::Range;

    use elide_core::modality::text::SourceRef;
    use elide_office::opc::OffsetMap;

    /// The raw source range(s), part-tagged, that a decoded-value `local` range
    /// in a block at `part` came from — via the block's offset map. The forward
    /// half; the encoder's `source_span` is a thin wrapper.
    pub(super) fn source_span(
        part: &str,
        offsets: &OffsetMap,
        local: Range<usize>,
    ) -> Vec<SourceRef> {
        offsets
            .raw_ranges(local)
            .into_iter()
            .map(|range| SourceRef::in_part(range, part))
            .collect()
    }

    /// Reverse of [`source_span`]: locate the block index and decoded-local range
    /// that raw `source` references address. `blocks` yields each block's part
    /// and offset map in item order. The references of one selection share a
    /// part and a block; the covered raw span (first start .. last end) is
    /// reverse-mapped through that block's offset map.
    pub(super) fn locate_source<'a>(
        blocks: impl Iterator<Item = (&'a str, Range<usize>, &'a OffsetMap)>,
        source: &[SourceRef],
    ) -> Option<(usize, Range<usize>)> {
        let part = source.first()?.part.as_deref()?;
        let raw_start = source.iter().map(|s| s.range.start).min()?;
        let raw_end = source.iter().map(|s| s.range.end).max()?;

        let (i, offsets) = blocks
            .enumerate()
            .find_map(|(i, (block_part, span, offsets))| {
                (block_part == part && span.start <= raw_start && raw_end <= span.end)
                    .then_some((i, offsets))
            })?;

        let decoded = offsets.decoded_ranges(raw_start..raw_end);
        Some((i, decoded.first()?.start..decoded.last()?.end))
    }
}
