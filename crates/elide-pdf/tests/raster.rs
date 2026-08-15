//! Raster redaction: pixel fill, fresh image-only PDF emission, and the
//! certificate — all pure logic, exercised without a PDFium runtime by
//! supplying hand-built page observations.
#![cfg(feature = "render")]

use elide_pdf::Pdf;
use elide_pdf::render::{Detection, Glyph, GlyphSource, PageObservation, PixelRect};
use lopdf::content::{Content, Operation};
use lopdf::{Document, Object, Stream, dictionary};

/// A minimal source PDF (its bytes are only hashed into the certificate).
fn source_pdf() -> Vec<u8> {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let content = Content {
        operations: vec![Operation::new("BT", vec![])],
    };
    let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
    let page_id = doc.add_object(dictionary! {
        "Type" => "Page", "Parent" => pages_id, "Contents" => content_id,
    });
    let pages = dictionary! {
        "Type" => "Pages", "Kids" => vec![page_id.into()], "Count" => 1,
        "MediaBox" => vec![0.into(), 0.into(), 10.into(), 10.into()],
    };
    doc.objects.insert(pages_id, Object::Dictionary(pages));
    let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog_id);
    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    out
}

/// A source PDF carrying an `/Info` dictionary and an XMP `/Metadata` stream,
/// both holding a secret — to prove the raster output does not carry them.
fn source_pdf_with_metadata() -> Vec<u8> {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let content = Content {
        operations: vec![Operation::new("BT", vec![])],
    };
    let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
    let page_id = doc.add_object(dictionary! {
        "Type" => "Page", "Parent" => pages_id, "Contents" => content_id,
    });
    let pages = dictionary! {
        "Type" => "Pages", "Kids" => vec![page_id.into()], "Count" => 1,
        "MediaBox" => vec![0.into(), 0.into(), 10.into(), 10.into()],
    };
    doc.objects.insert(pages_id, Object::Dictionary(pages));
    let metadata_id = doc.add_object(Stream::new(
        dictionary! { "Type" => "Metadata", "Subtype" => "XML" },
        b"<xmp>secret-author: Confidential Person</xmp>".to_vec(),
    ));
    let catalog_id = doc.add_object(dictionary! {
        "Type" => "Catalog", "Pages" => pages_id, "Metadata" => metadata_id,
    });
    doc.trailer.set("Root", catalog_id);
    let info_id = doc.add_object(dictionary! {
        "Author" => Object::string_literal("Confidential Person"),
        "Title" => Object::string_literal("Top Secret Memo"),
    });
    doc.trailer.set("Info", info_id);
    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    out
}

/// A 4x1 all-white RGB8 page with two glyphs: "AB" at pixels [0,2), "CD" at
/// [2,4). UTF-16 offsets: A=0,B=1,C=2,D=3.
fn observation() -> PageObservation {
    let white = vec![255u8; 4 * 3]; // 4x1 RGB8
    PageObservation {
        page: 1,
        width: 4,
        height: 1,
        text: "ABCD".to_string(),
        glyphs: vec![
            Glyph {
                start: 0,
                end: 1,
                rect: PixelRect::new(0, 0, 1, 1),
                source: GlyphSource::Text,
            },
            Glyph {
                start: 1,
                end: 2,
                rect: PixelRect::new(1, 0, 1, 1),
                source: GlyphSource::Text,
            },
            Glyph {
                start: 2,
                end: 3,
                rect: PixelRect::new(2, 0, 1, 1),
                source: GlyphSource::Text,
            },
            Glyph {
                start: 3,
                end: 4,
                rect: PixelRect::new(3, 0, 1, 1),
                source: GlyphSource::Text,
            },
        ],
        pixels: white,
    }
}

/// Decode the single image XObject's RGB8 samples out of an emitted PDF.
fn output_image_pixels(out: &[u8]) -> Vec<u8> {
    let doc = Document::load_mem(out).unwrap();
    let stream = doc
        .objects
        .values()
        .find_map(|o| {
            let s = o.as_stream().ok()?;
            (s.dict.get(b"Subtype").ok()?.as_name().ok()? == b"Image").then_some(s)
        })
        .expect("output has an image XObject");
    // The image stream is FlateDecode-compressed; decode back to raw samples.
    stream.decompressed_content().unwrap()
}

#[test]
fn fills_only_detected_glyph_pixels() {
    let pdf = Pdf::open(&source_pdf()).unwrap();
    // Redact "BC" — UTF-16 [1, 3): the middle two pixels of the 4x1 page.
    let (out, _cert) = pdf
        .redact_raster(vec![observation()], &[Detection::new(1, 1, 3)], [0, 0, 0])
        .unwrap();

    // Decode the emitted image and check the actual filled pixels.
    let pixels = output_image_pixels(&out);
    assert_eq!(&pixels[0..3], &[255, 255, 255], "A untouched");
    assert_eq!(&pixels[3..6], &[0, 0, 0], "B redacted");
    assert_eq!(&pixels[6..9], &[0, 0, 0], "C redacted");
    assert_eq!(&pixels[9..12], &[255, 255, 255], "D untouched");
}

#[test]
fn output_is_a_fresh_image_only_pdf() {
    let pdf = Pdf::open(&source_pdf()).unwrap();
    let (out, _cert) = pdf
        .redact_raster(vec![observation()], &[Detection::new(1, 0, 4)], [0, 0, 0])
        .unwrap();

    // The output parses, has one page, and its page draws an image XObject.
    let doc = Document::load_mem(&out).unwrap();
    assert_eq!(doc.get_pages().len(), 1);
    let has_image = doc.objects.values().any(|o| {
        o.as_stream()
            .ok()
            .and_then(|s| s.dict.get(b"Subtype").ok())
            .and_then(|v| v.as_name().ok())
            == Some(b"Image")
    });
    assert!(has_image, "output must contain an image XObject");

    // No text-drawing operator survives: the output carries no extractable text.
    let extraction = Pdf::open(&out).unwrap().extract();
    let text: String = extraction
        .blocks
        .iter()
        .map(|b| b.text.to_string())
        .collect();
    assert!(
        text.trim().is_empty(),
        "output must have no text layer: {text:?}"
    );
}

#[test]
fn output_drops_source_metadata() {
    let source = source_pdf_with_metadata();
    // Sanity: the source really does carry the secret in its metadata.
    assert!(String::from_utf8_lossy(&source).contains("Confidential Person"));

    let pdf = Pdf::open(&source).unwrap();
    let (out, _cert) = pdf
        .redact_raster(vec![observation()], &[Detection::new(1, 0, 4)], [0, 0, 0])
        .unwrap();

    // The fresh image-only output copies nothing from the source: no `/Info`
    // entries, no `/Metadata` stream, and none of the secret bytes survive.
    let inspection = Pdf::open(&out).unwrap().inspect().unwrap();
    assert_eq!(inspection.risks.document_info_entry_count, 0);
    assert_eq!(inspection.risks.metadata_stream_count, 0);
    let raw = String::from_utf8_lossy(&out);
    assert!(
        !raw.contains("Confidential Person"),
        "author metadata survived"
    );
    assert!(!raw.contains("Top Secret Memo"), "title metadata survived");
}

#[test]
fn certificate_binds_source_pages_and_output() {
    let source = source_pdf();
    let pdf = Pdf::open(&source).unwrap();
    let (out, cert) = pdf
        .redact_raster(vec![observation()], &[Detection::new(1, 0, 2)], [0, 0, 0])
        .unwrap();

    // 64 hex chars per SHA-256; one page digest; output digest matches `out`.
    assert_eq!(cert.source_sha256.len(), 64);
    assert_eq!(cert.page_sha256.len(), 1);
    assert_eq!(cert.output_sha256.len(), 64);
    // The output digest is over the returned bytes.
    use sha2::{Digest, Sha256};
    let expect: String = Sha256::digest(&out)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(cert.output_sha256, expect);
}

#[test]
fn rejects_a_mismatched_pixel_buffer() {
    let pdf = Pdf::open(&source_pdf()).unwrap();
    let mut obs = observation();
    obs.pixels.truncate(5); // a buffer that is not width*height*3
    let err = pdf.redact_raster(vec![obs], &[], [0, 0, 0]).unwrap_err();
    assert_eq!(err.kind(), elide_pdf::ErrorKind::UnsafeRewrite);
}

#[test]
fn rejects_a_detection_on_an_absent_page() {
    let pdf = Pdf::open(&source_pdf()).unwrap();
    let err = pdf
        .redact_raster(vec![observation()], &[Detection::new(9, 0, 1)], [0, 0, 0])
        .unwrap_err();
    assert_eq!(err.kind(), elide_pdf::ErrorKind::UnsafeRewrite);
}

/// Fill the detected glyph boxes on `pages` in place with `fill`, the same
/// pixel edit `redact_raster` performs before emitting — so a caller can then
/// assert the fill actually covered every detected region.
#[cfg(feature = "test-utils")]
fn fill_detected(pages: &mut [PageObservation], detections: &[Detection], fill: [u8; 3]) {
    for page in pages.iter_mut() {
        for det in detections.iter().filter(|d| d.page == page.page) {
            for glyph in page
                .glyphs
                .iter()
                .filter(|g| g.start < det.end && g.end > det.start)
            {
                let rect = glyph.rect;
                for y in rect.y..rect.y.saturating_add(rect.height).min(page.height) {
                    for x in rect.x..rect.x.saturating_add(rect.width).min(page.width) {
                        let i = ((y as usize) * (page.width as usize) + x as usize) * 3;
                        page.pixels[i..i + 3].copy_from_slice(&fill);
                    }
                }
            }
        }
    }
}

#[cfg(feature = "test-utils")]
#[test]
fn verify_raster_coverage_accepts_a_fully_filled_page() {
    use elide_pdf::render::verify_raster_coverage;

    let fill = [0, 0, 0];
    let detections = [Detection::new(1, 1, 3)];
    let mut pages = vec![observation()];
    fill_detected(&mut pages, &detections, fill);

    // Every detected glyph box is painted, so verification passes.
    verify_raster_coverage(&pages, &detections, fill).expect("filled regions verify");
}

#[cfg(feature = "test-utils")]
#[test]
fn verify_raster_coverage_catches_an_unpainted_region() {
    use elide_pdf::render::verify_raster_coverage;

    // The observation is never filled, so a detection's box still shows the
    // original white pixels — verification must fail closed.
    let err = verify_raster_coverage(&[observation()], &[Detection::new(1, 1, 3)], [0, 0, 0])
        .unwrap_err();
    assert_eq!(err.kind(), elide_pdf::ErrorKind::UnsafeRewrite);
}
