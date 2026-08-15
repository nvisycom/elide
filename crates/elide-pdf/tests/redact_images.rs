//! Image redaction: replace an embedded image XObject with a redacted image,
//! rebuilding a valid XObject from encoded bytes. Requires the `image` feature.
#![cfg(feature = "image")]

use elide_pdf::Pdf;
use elide_pdf::extract::ImageId;
use elide_pdf::redact::ImageReplacement;
use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document, Object, Stream, dictionary};

/// A one-page PDF with one embedded image XObject (a raw RGB image). Returns the
/// document bytes and the image's id.
fn image_pdf(pixels: &[u8], w: i64, h: i64) -> (Vec<u8>, (u32, u16)) {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();

    let mut img = Dictionary::new();
    img.set("Type", Object::Name(b"XObject".to_vec()));
    img.set("Subtype", Object::Name(b"Image".to_vec()));
    img.set("Width", Object::Integer(w));
    img.set("Height", Object::Integer(h));
    img.set("ColorSpace", Object::Name(b"DeviceRGB".to_vec()));
    img.set("BitsPerComponent", Object::Integer(8));
    let image_id = doc.add_object(Stream::new(img, pixels.to_vec()));

    let resources_id = doc.add_object(dictionary! {
        "XObject" => dictionary! { "Im1" => image_id },
    });
    let content = Content {
        operations: vec![Operation::new("Do", vec![Object::Name(b"Im1".to_vec())])],
    };
    let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
    let page_id = doc.add_object(dictionary! {
        "Type" => "Page", "Parent" => pages_id,
        "Contents" => content_id, "Resources" => resources_id,
    });
    let pages = dictionary! {
        "Type" => "Pages", "Kids" => vec![page_id.into()], "Count" => 1,
        "MediaBox" => vec![0.into(), 0.into(), 100.into(), 100.into()],
    };
    doc.objects.insert(pages_id, Object::Dictionary(pages));
    let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog_id);

    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    (out, image_id)
}

/// A solid black 2x2 PNG (the "redacted" image).
fn black_png() -> Vec<u8> {
    let img = image::RgbImage::from_pixel(2, 2, image::Rgb([0, 0, 0]));
    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(img)
        .write_to(&mut out, image::ImageFormat::Png)
        .unwrap();
    out.into_inner()
}

#[test]
fn replaces_an_image_with_a_redacted_one() {
    // Original: 2x2 white image.
    let white = vec![255u8; 2 * 2 * 3];
    let (pdf, id) = image_pdf(&white, 2, 2);

    let extraction = Pdf::open(&pdf).unwrap().extract();
    assert_eq!(extraction.embeddings.len(), 1);
    let image_id = ImageId::new(id.0, id.1);

    let out = Pdf::open(&pdf)
        .unwrap()
        .redact_images(&[ImageReplacement {
            id: image_id,
            image: black_png(),
        }])
        .unwrap();

    // The output re-parses and still has exactly one embedded image.
    let after = Pdf::open(&out).unwrap().extract();
    assert_eq!(after.embeddings.len(), 1);

    // The image's stream bytes changed: the original white samples are gone.
    let emb = &after.embeddings[0];
    assert_ne!(
        emb.bytes.as_ref(),
        white.as_slice(),
        "image was not replaced"
    );
}

#[test]
fn is_fail_closed_on_a_non_image_object() {
    let white = vec![255u8; 2 * 2 * 3];
    let (pdf, _) = image_pdf(&white, 2, 2);
    let err = Pdf::open(&pdf)
        .unwrap()
        .redact_images(&[ImageReplacement {
            id: ImageId::new(9999, 0), // no such object
            image: black_png(),
        }])
        .unwrap_err();
    assert_eq!(err.kind(), elide_pdf::ErrorKind::UnsafeRewrite);
}

#[test]
fn is_fail_closed_on_undecodable_image_bytes() {
    let white = vec![255u8; 2 * 2 * 3];
    let (pdf, id) = image_pdf(&white, 2, 2);
    let err = Pdf::open(&pdf)
        .unwrap()
        .redact_images(&[ImageReplacement {
            id: ImageId::new(id.0, id.1),
            image: b"not a real image".to_vec(),
        }])
        .unwrap_err();
    assert_eq!(err.kind(), elide_pdf::ErrorKind::UnsafeRewrite);
}

/// A one-page PDF with an image XObject that has a soft mask (`/SMask`)
/// referencing a second image object. Returns the bytes, the main image id, and
/// the SMask object id.
fn image_pdf_with_smask() -> (Vec<u8>, (u32, u16), (u32, u16)) {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();

    // The soft mask: a 2x2 8-bit grayscale image (the "sensitive shape").
    let mut mask = Dictionary::new();
    mask.set("Type", Object::Name(b"XObject".to_vec()));
    mask.set("Subtype", Object::Name(b"Image".to_vec()));
    mask.set("Width", Object::Integer(2));
    mask.set("Height", Object::Integer(2));
    mask.set("ColorSpace", Object::Name(b"DeviceGray".to_vec()));
    mask.set("BitsPerComponent", Object::Integer(8));
    let smask_id = doc.add_object(Stream::new(mask, b"SECRETMK".to_vec()));

    let mut img = Dictionary::new();
    img.set("Type", Object::Name(b"XObject".to_vec()));
    img.set("Subtype", Object::Name(b"Image".to_vec()));
    img.set("Width", Object::Integer(2));
    img.set("Height", Object::Integer(2));
    img.set("ColorSpace", Object::Name(b"DeviceRGB".to_vec()));
    img.set("BitsPerComponent", Object::Integer(8));
    img.set("SMask", Object::Reference(smask_id));
    let image_id = doc.add_object(Stream::new(img, vec![255u8; 12]));

    let res = doc.add_object(dictionary! { "XObject" => dictionary! { "Im1" => image_id } });
    let content = Content {
        operations: vec![Operation::new("Do", vec![Object::Name(b"Im1".to_vec())])],
    };
    let cid = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
    let page = doc.add_object(dictionary! {
        "Type" => "Page", "Parent" => pages_id, "Contents" => cid, "Resources" => res,
    });
    let pages = dictionary! {
        "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1,
        "MediaBox" => vec![0.into(), 0.into(), 100.into(), 100.into()],
    };
    doc.objects.insert(pages_id, Object::Dictionary(pages));
    let cat = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", cat);
    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    (out, image_id, smask_id)
}

#[test]
fn deletes_the_orphaned_soft_mask() {
    let (pdf, image_id, _smask_id) = image_pdf_with_smask();
    // Sanity: the source carries the mask's sensitive bytes.
    assert!(String::from_utf8_lossy(&pdf).contains("SECRETMK"));

    let out = Pdf::open(&pdf)
        .unwrap()
        .redact_images(&[ImageReplacement {
            id: ImageId::new(image_id.0, image_id.1),
            image: black_png(),
        }])
        .unwrap();

    // The soft mask's bytes must not survive: replacing the image without
    // deleting its /SMask would leave the mask orphaned but serialised.
    assert!(
        !String::from_utf8_lossy(&out).contains("SECRETMK"),
        "the orphaned soft mask survived in the output"
    );
    // And the output still re-parses with one image.
    assert_eq!(Pdf::open(&out).unwrap().extract().embeddings.len(), 1);
}
