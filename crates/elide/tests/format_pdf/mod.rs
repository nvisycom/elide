//! PDF scenarios: `text` (pure-Rust born-digital splice), `raster`
//! (PDFium-rendered image-only redaction, `#[ignore]`d without PDFium), and,
//! through the codec alone, `images` (embedded images surfaced, redacted, or
//! removed) and `sanitize` (hidden content stripped at decode).
#![allow(dead_code)]

mod images;
mod marked;
mod raster;
mod sanitize;
mod text;

use bytes::Bytes;
use elide::codec::pdf::{RasterMode, pdf_format_with};
use elide::codec::{ContentData, Document};
use elide::redaction::HiddenContentPolicy;

/// Decode `pdf` under `policy` on the glyph-deletion path. A synthetic page
/// without text would otherwise be rendered as a scanned page under the
/// default `Auto` mode, pulling in PDFium for nothing.
pub(super) async fn decode(pdf: Bytes, policy: HiddenContentPolicy) -> Document {
    pdf_format_with(RasterMode::Never)
        .decode(ContentData::new(pdf), policy)
        .await
        .expect("decode")
}
