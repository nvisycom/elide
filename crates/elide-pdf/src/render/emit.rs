//! Build a fresh image-only PDF from sanitised page pixels, and the
//! [`Certificate`] that binds it to its source.

use lopdf::{Object, Stream, dictionary};
use sha2::{Digest, Sha256};

use super::PageObservation;
use crate::error::{Error, Result};

/// Verifiable provenance of a raster redaction: the SHA-256 of the source, of
/// each page's sanitised pixels, and of the emitted output.
///
/// The digests let a caller prove the output was derived from this exact source
/// by this exact sanitisation, and that neither was altered afterwards.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Certificate {
    /// Lowercase hex SHA-256 of the source document bytes.
    pub source_sha256: String,
    /// Lowercase hex SHA-256 of each page's sanitised RGB8 pixels, in page
    /// order.
    pub page_sha256: Vec<String>,
    /// Lowercase hex SHA-256 of the emitted output document bytes.
    pub output_sha256: String,
}

/// Lowercase hex SHA-256 of `bytes`.
fn digest_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;

    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        // Write straight into the pre-reserved buffer; no per-byte allocation.
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// Emit a fresh image-only PDF from `pages`, copying nothing from the source.
///
/// Each page becomes: an image XObject (DeviceRGB, 8bpc, FlateDecode) holding
/// the sanitised pixels, a content stream that draws it to fill the page, and a
/// page object sized to the pixels. The document is rebuilt from scratch — a
/// new Catalog and page tree — so no source object, metadata, or prior revision
/// survives.
pub(super) fn emit(source: &[u8], pages: Vec<PageObservation>) -> Result<(Vec<u8>, Certificate)> {
    let mut doc = lopdf::Document::with_version("1.7");
    let pages_id = doc.new_object_id();

    let mut page_ids = Vec::with_capacity(pages.len());
    let mut page_sha256 = Vec::with_capacity(pages.len());

    for page in &pages {
        page_sha256.push(digest_hex(&page.pixels));

        // The image XObject: raw RGB8 samples, FlateDecode-compressed.
        let mut image = Stream::new(
            dictionary! {
                "Type" => "XObject",
                "Subtype" => "Image",
                "Width" => page.width as i64,
                "Height" => page.height as i64,
                "ColorSpace" => "DeviceRGB",
                "BitsPerComponent" => 8,
            },
            page.pixels.clone(),
        );
        image
            .compress()
            .map_err(|e| Error::invalid_document(format!("failed to compress page image: {e}")))?;
        let image_id = doc.add_object(image);

        // Draw the image to fill the whole page: `q W 0 0 H cm /Im Do Q`, with
        // the page sized 1 unit per pixel so the image maps 1:1.
        let w = page.width;
        let h = page.height;
        let content = format!("q {w} 0 0 {h} 0 0 cm /Im0 Do Q");
        let content_id = doc.add_object(Stream::new(dictionary! {}, content.into_bytes()));

        let resources_id = doc.add_object(dictionary! {
            "XObject" => dictionary! { "Im0" => Object::Reference(image_id) },
        });
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => Object::Reference(pages_id),
            "Contents" => Object::Reference(content_id),
            "Resources" => Object::Reference(resources_id),
            "MediaBox" => vec![0.into(), 0.into(), (w as i64).into(), (h as i64).into()],
        });
        page_ids.push(page_id);
    }

    let kids: Vec<Object> = page_ids.iter().map(|&id| Object::Reference(id)).collect();
    let count = kids.len() as i64;
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => kids,
            "Count" => count,
        }),
    );
    let catalog_id = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => Object::Reference(pages_id),
    });
    doc.trailer.set("Root", Object::Reference(catalog_id));

    let mut out = Vec::new();
    doc.save_to(&mut out)
        .map_err(|e| Error::invalid_document(format!("failed to save redacted PDF: {e}")))?;

    let certificate = Certificate {
        source_sha256: digest_hex(source),
        page_sha256,
        output_sha256: digest_hex(&out),
    };
    Ok((out, certificate))
}
