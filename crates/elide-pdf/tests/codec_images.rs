//! Embedded images through the codec: surfaced as blobs, redacted, written
//! back, or removed.

use bytes::Bytes;
use elide_codec::content::ContentData;
use elide_codec::{DocumentPart, LocalId};
use elide_core::redaction::{HiddenContent, HiddenContentPolicy};
use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document, Object, Stream, dictionary};

mod common;

use common::{format, streams};

/// A JPEG-encoded image of a solid colour, as bytes (a self-contained
/// `.jpg` file, which the codec surfaces as it is).
fn jpeg(rgb: [u8; 3]) -> Vec<u8> {
    let img = image::RgbImage::from_pixel(8, 8, image::Rgb(rgb));
    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(img)
        .write_to(&mut out, image::ImageFormat::Jpeg)
        .unwrap();
    out.into_inner()
}

/// An image XObject dictionary of `width` x `height` 8-bit RGB samples
/// stored with `filter`.
fn image_dict(width: i64, height: i64, filter: &str) -> Dictionary {
    let mut img = Dictionary::new();
    img.set("Type", Object::Name(b"XObject".to_vec()));
    img.set("Subtype", Object::Name(b"Image".to_vec()));
    img.set("Width", Object::Integer(width));
    img.set("Height", Object::Integer(height));
    img.set("ColorSpace", Object::Name(b"DeviceRGB".to_vec()));
    img.set("BitsPerComponent", Object::Integer(8));
    img.set("Filter", Object::Name(filter.as_bytes().to_vec()));
    img
}

/// A one-page PDF with a single embedded JPEG (`DCTDecode`) image XObject,
/// the kind whose raw stream bytes are a decodable file. Returns the
/// document bytes and the image's `(number, generation)` id.
fn image_pdf() -> (Vec<u8>, (u32, u16)) {
    let image = Stream::new(image_dict(8, 8, "DCTDecode"), jpeg([255, 255, 255]));
    pdf_drawing(vec![image], Vec::new())
}

/// A one-page PDF that draws each of `images` (named `Im1`, `Im2`, …),
/// then runs the `extra` operations. Returns the document bytes and the
/// first image's `(number, generation)` id.
fn pdf_drawing(images: Vec<Stream>, extra: Vec<Operation>) -> (Vec<u8>, (u32, u16)) {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let mut xobjects = Dictionary::new();
    let mut operations = Vec::new();
    let mut first = None;
    for (index, image) in images.into_iter().enumerate() {
        let id = doc.add_object(image);
        first.get_or_insert(id);
        let name = format!("Im{}", index + 1);
        xobjects.set(name.as_bytes(), id);
        operations.push(Operation::new("Do", vec![Object::Name(name.into_bytes())]));
    }
    operations.extend(extra);
    let image_id = first.expect("an image");
    let res = doc.add_object(dictionary! { "XObject" => xobjects });
    let content = Content { operations };
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
    (out, image_id)
}

fn black_png() -> Vec<u8> {
    let img = image::RgbImage::from_pixel(2, 2, image::Rgb([0, 0, 0]));
    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(img)
        .write_to(&mut out, image::ImageFormat::Png)
        .unwrap();
    out.into_inner()
}

/// The image blob parts a document surfaces, as `(id, bytes)`.
/// The embedded-image blobs (`img-*`) the document surfaces, excluding any
/// scanned-page (`page-*`) blobs the render path also adds.
fn image_blobs(doc: &elide_codec::Document) -> Vec<(String, Bytes)> {
    doc.parts()
        .iter()
        .filter_map(|part| match part {
            DocumentPart::Blob { id, bytes, .. } if id.as_str().starts_with("img-") => {
                Some((id.as_str().to_owned(), bytes.clone()))
            }
            _ => None,
        })
        .collect()
}

/// The PNG signature every surfaced raw-sample image starts with.
const PNG: &[u8] = b"\x89PNG";

/// Raw samples (a lossless image, or a page this crate flattened) surface
/// as a PNG, and a redaction of one is written back and passes the check.
#[tokio::test]
async fn raw_samples_surface_as_png_and_round_trip() {
    let mut raw = Stream::new(image_dict(2, 2, "FlateDecode"), vec![200; 12]);
    raw.dict.remove(b"Filter");
    raw.compress().unwrap();
    let (pdf, _) = pdf_drawing(vec![raw], Vec::new());
    let mut doc = format()
        .decode(
            ContentData::new(Bytes::from(pdf)),
            HiddenContentPolicy::default(),
        )
        .await
        .expect("decode");
    let blobs = image_blobs(&doc);
    assert_eq!(blobs.len(), 1);
    assert!(blobs[0].1.starts_with(PNG));

    doc.replace_part(&LocalId::new(blobs[0].0.clone()), Bytes::from(black_png()))
        .unwrap();
    let encoded = doc.encode().expect("a redacted raw image passes the check");
    assert_eq!(streams(encoded.as_bytes(), b"Image").len(), 1);
}

/// What cannot be surfaced (fax samples, an inline image) is removed at
/// decode, and an image no codec could read is removed on encode, its
/// drawing cut rather than blanked.
#[tokio::test]
async fn unsurfaceable_and_unreadable_images_are_removed() {
    let fax = Stream::new(image_dict(8, 8, "CCITTFaxDecode"), vec![0; 4]);
    let jpeg = Stream::new(image_dict(8, 8, "DCTDecode"), jpeg([255, 255, 255]));
    let inline = Operation::new(
        "BI",
        vec![Object::Stream(Stream::new(
            dictionary! { "W" => 1, "H" => 1, "BPC" => 8, "CS" => "G" },
            vec![0],
        ))],
    );
    let (pdf, _) = pdf_drawing(vec![fax, jpeg], vec![inline]);
    let mut doc = format()
        .decode(
            ContentData::new(Bytes::from(pdf)),
            HiddenContentPolicy::default(),
        )
        .await
        .expect("decode");
    let unreadable: usize = doc
        .stripped()
        .iter()
        .filter(|entry| entry.content == HiddenContent::Unreadable)
        .map(|entry| entry.count)
        .sum();
    assert_eq!(unreadable, 2);

    let blobs = image_blobs(&doc);
    assert_eq!(blobs.len(), 1);
    doc.remove_part(&LocalId::new(blobs[0].0.clone())).unwrap();
    let encoded = doc.encode().expect("encode");
    assert!(streams(encoded.as_bytes(), b"Image").is_empty());
    let saved = Document::load_mem(encoded.as_bytes()).unwrap();
    let page = saved.get_pages().into_values().next().unwrap();
    let drawn = String::from_utf8_lossy(&saved.get_page_content(page)).into_owned();
    assert!(!drawn.contains("Do") && !drawn.contains("BI"), "{drawn}");
}

#[tokio::test]
async fn document_surfaces_and_replaces_an_image() {
    let (pdf, image_id) = image_pdf();
    let mut doc = format()
        .decode(
            ContentData::new(Bytes::from(pdf)),
            HiddenContentPolicy::default(),
        )
        .await
        .unwrap();

    // The document surfaces the one embedded image as a blob part.
    let blobs = image_blobs(&doc);
    assert_eq!(blobs.len(), 1);
    assert_eq!(blobs[0].0, format!("img-{}-{}", image_id.0, image_id.1));

    // Fold a redacted (black) image back into the document.
    doc.replace_part(&LocalId::new(blobs[0].0.clone()), Bytes::from(black_png()))
        .unwrap();

    // Encode applies it: the output's image bytes differ from the original.
    let original = jpeg([255, 255, 255]);
    let encoded = doc.encode().unwrap();
    let after = streams(encoded.as_bytes(), b"Image");
    assert_eq!(after.len(), 1);
    assert_ne!(after[0], original, "image was not redacted");
}

#[tokio::test]
async fn replace_part_rejects_an_unknown_id() {
    let (pdf, _) = image_pdf();
    let mut doc = format()
        .decode(
            ContentData::new(Bytes::from(pdf)),
            HiddenContentPolicy::default(),
        )
        .await
        .unwrap();
    let err = doc
        .replace_part(&LocalId::new("img-9999-0"), Bytes::from(black_png()))
        .unwrap_err();
    assert_eq!(err.kind(), elide_core::ErrorKind::MalformedInput);
}
