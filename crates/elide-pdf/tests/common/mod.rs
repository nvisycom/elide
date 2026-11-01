//! Shared helpers for the PDF codec's end-to-end tests.

use elide_codec::Format;
#[cfg(feature = "render")]
use elide_pdf::primitive::RasterMode;
use lopdf::{Document, Object};

/// The PDF format on the glyph-deletion path. A synthetic page without text
/// would otherwise be rendered as a scanned page under the default `Auto`
/// mode, pulling in PDFium for nothing.
pub fn format() -> Format {
    #[cfg(feature = "render")]
    {
        elide_pdf::codec::pdf_format_with(RasterMode::Never)
    }
    #[cfg(not(feature = "render"))]
    {
        elide_pdf::codec::pdf_format()
    }
}

/// The stream contents of every object of `pdf` whose dictionary says it is
/// `kind`: an image XObject (`/Subtype /Image`) or an embedded file
/// (`/Type /EmbeddedFile`).
pub fn streams(pdf: &[u8], kind: &[u8]) -> Vec<Vec<u8>> {
    let doc = Document::load_mem(pdf).expect("a readable PDF");
    doc.objects
        .values()
        .filter_map(|object| match object {
            Object::Stream(stream) => Some(stream),
            _ => None,
        })
        .filter(|stream| {
            let named = |key: &[u8]| {
                stream
                    .dict
                    .get(key)
                    .and_then(Object::as_name)
                    .is_ok_and(|name| name == kind)
            };
            named(b"Subtype") || named(b"Type")
        })
        .map(|stream| stream.content.clone())
        .collect()
}
