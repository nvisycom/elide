//! Page reflatten (feature `image`): replace a scanned/image-only page with a
//! redacted image while leaving born-digital text pages intact.

#![cfg(feature = "image")]

use elide_pdf::Pdf;
use elide_pdf::redact::PageReplacement;

/// A tiny solid-colour PNG of `w`x`h`, standing in for a redacted page raster.
fn png(w: u32, h: u32, colour: [u8; 3]) -> Vec<u8> {
    use std::io::Cursor;

    let buf = image::RgbImage::from_pixel(w, h, image::Rgb(colour));
    let mut out = Vec::new();
    image::DynamicImage::ImageRgb8(buf)
        .write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
    out
}

/// A two-page PDF: page 1 draws real text ("KEEP THIS TEXT"), page 2 is an
/// image-only page whose content stream draws an embedded image and carries a
/// secret string in a comment, so we can tell whether it survives reflatten.
fn two_page_doc() -> Vec<u8> {
    use lopdf::content::{Content, Operation};
    use lopdf::{Document, Object, Stream, dictionary};

    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();

    // Page 1: a text page.
    let font_id = doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Courier",
    });
    let res1 = doc.add_object(dictionary! { "Font" => dictionary! { "F1" => font_id } });
    let body1 = Content {
        operations: vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 12.into()]),
            Operation::new("Td", vec![72.into(), 700.into()]),
            Operation::new("Tj", vec![Object::string_literal("KEEP THIS TEXT")]),
            Operation::new("ET", vec![]),
        ],
    };
    let content1 = doc.add_object(Stream::new(dictionary! {}, body1.encode().unwrap()));
    let page1 = doc.add_object(dictionary! {
        "Type" => "Page", "Parent" => pages_id,
        "Contents" => content1, "Resources" => res1,
        "MediaBox" => vec![0.into(), 0.into(), 300.into(), 400.into()],
    });

    // Page 2: an image-only page. Its content stream carries a marker so we can
    // check the original page content is gone after reflatten.
    let img = Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Image",
            "Width" => 2, "Height" => 2, "ColorSpace" => "DeviceGray",
            "BitsPerComponent" => 8,
        },
        b"ORIGINAL-SCAN-PIXELS".to_vec(),
    );
    let img_id = doc.add_object(img);
    let res2 = doc.add_object(dictionary! {
        "XObject" => dictionary! { "Im0" => Object::Reference(img_id) },
    });
    let content2 = doc.add_object(Stream::new(
        dictionary! {},
        b"q 300 0 0 400 0 0 cm /Im0 Do Q % SECRET-PAGE-MARKER".to_vec(),
    ));
    let page2 = doc.add_object(dictionary! {
        "Type" => "Page", "Parent" => pages_id,
        "Contents" => content2, "Resources" => res2,
        "MediaBox" => vec![0.into(), 0.into(), 300.into(), 400.into()],
    });

    let pages = dictionary! {
        "Type" => "Pages", "Kids" => vec![page1.into(), page2.into()], "Count" => 2,
    };
    doc.objects.insert(pages_id, Object::Dictionary(pages));
    let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog_id);

    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    out
}

/// The concatenated text of every page.
fn extracted(pdf: &[u8]) -> String {
    Pdf::open(pdf)
        .unwrap()
        .extract()
        .blocks
        .iter()
        .map(|b| b.text.to_string())
        .collect()
}

#[test]
fn reflattens_only_the_named_page() {
    let doc = two_page_doc();
    // The original scan pixels and page-2 marker are present up front.
    assert!(String::from_utf8_lossy(&doc).contains("ORIGINAL-SCAN-PIXELS"));
    assert!(String::from_utf8_lossy(&doc).contains("SECRET-PAGE-MARKER"));

    // Reflatten page 2 with a redacted raster; page 1 is left alone.
    let redacted = png(300, 400, [0, 0, 0]);
    let out = Pdf::open(&doc)
        .unwrap()
        .redact_pages(&[PageReplacement {
            number: 2,
            image: redacted,
        }])
        .unwrap();

    // Page 1's text survives (born-digital page untouched).
    assert!(extracted(&out).contains("KEEP THIS TEXT"), "text page lost");

    // Page 2's original content is gone: neither the source pixels nor the
    // content-stream marker remain in the output bytes.
    let raw = String::from_utf8_lossy(&out);
    assert!(
        !raw.contains("ORIGINAL-SCAN-PIXELS"),
        "original scan pixels survived reflatten"
    );
    assert!(
        !raw.contains("SECRET-PAGE-MARKER"),
        "original page content survived reflatten"
    );

    // The output still opens with two pages and exactly one text block (page 1).
    let reopened = Pdf::open(&out).unwrap();
    assert_eq!(reopened.extract().blocks.len(), 1, "expected one text page");
    assert_eq!(reopened.inspect().unwrap().page_count, 2);
}

#[test]
fn is_fail_closed_on_a_missing_page() {
    let doc = two_page_doc();
    let err = Pdf::open(&doc)
        .unwrap()
        .redact_pages(&[PageReplacement {
            number: 99,
            image: png(2, 2, [0, 0, 0]),
        }])
        .expect_err("missing page should be refused");
    assert_eq!(err.kind(), elide_pdf::ErrorKind::UnsafeRewrite);
}

#[test]
fn is_fail_closed_on_undecodable_image() {
    let doc = two_page_doc();
    let err = Pdf::open(&doc)
        .unwrap()
        .redact_pages(&[PageReplacement {
            number: 2,
            image: b"not an image".to_vec(),
        }])
        .expect_err("undecodable image should be refused");
    assert_eq!(err.kind(), elide_pdf::ErrorKind::UnsafeRewrite);
}
