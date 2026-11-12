//! Small in-memory PDF fixtures, behind the `fixtures` feature.
//!
//! A downstream crate testing the PDF codec needs real documents carrying
//! embedded images, attachments, and metadata, and a way to check what an
//! encoded document still holds, without taking a direct dependency on
//! `lopdf` or `image`. These helpers build and inspect such fixtures so the
//! test stays in this crate's vocabulary.

use std::io::Cursor;

use ::image::{DynamicImage, ImageFormat, Rgb, RgbImage};
use bytes::Bytes;
use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document, Object, Stream, dictionary};

/// A one-page PDF drawing one 8x8 JPEG image (`DCTDecode`).
///
/// The kind whose stream is a file the image pipeline reads as it is.
#[must_use]
pub fn jpeg_image() -> Bytes {
    let image = Stream::new(image_dict(8, 8, Some("DCTDecode")), jpeg());
    drawing(vec![image], Vec::new())
}

/// A one-page PDF drawing one 2x2 image of raw RGB samples (`FlateDecode`).
///
/// The way every lossless image is stored.
#[must_use]
pub fn raw_image() -> Bytes {
    let mut image = Stream::new(image_dict(2, 2, None), vec![200; 12]);
    image.compress().expect("compress raw samples");
    drawing(vec![image], Vec::new())
}

/// A one-page PDF drawing three images, two the image pipeline cannot read.
///
/// A fax-coded image (`CCITTFaxDecode`), a JPEG image, and an inline image; the
/// JPEG is the one it can read.
#[must_use]
pub fn unsurfaceable_images() -> Bytes {
    let fax = Stream::new(image_dict(8, 8, Some("CCITTFaxDecode")), vec![0; 4]);
    let jpeg = Stream::new(image_dict(8, 8, Some("DCTDecode")), jpeg());
    let inline = Operation::new(
        "BI",
        vec![Object::Stream(Stream::new(
            dictionary! { "W" => 1, "H" => 1, "BPC" => 8, "CS" => "G" },
            vec![0],
        ))],
    );
    drawing(vec![fax, jpeg], vec![inline])
}

/// A one-page PDF with an author in `/Info` and one attached file.
///
/// Its `/Info` names its author, `Jane Roe`, and it attaches one file,
/// `notes.txt`, holding `Jane Roe`.
#[must_use]
pub fn metadata_and_attachment() -> Bytes {
    let mut doc = Document::with_version("1.7");
    let pages = doc.new_object_id();
    let page = doc.add_object(dictionary! { "Type" => "Page", "Parent" => pages });
    doc.objects.insert(
        pages,
        Object::Dictionary(
            dictionary! { "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1 },
        ),
    );
    let file = doc.add_object(Stream::new(
        dictionary! { "Type" => "EmbeddedFile" },
        b"Jane Roe".to_vec(),
    ));
    let spec = doc.add_object(dictionary! {
        "Type" => "Filespec",
        "F" => Object::string_literal("notes.txt"),
        "EF" => dictionary! { "F" => file },
    });
    let catalog = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages,
        "Names" => dictionary! {
            "EmbeddedFiles" => dictionary! {
                "Names" => vec![Object::string_literal("notes.txt"), spec.into()],
            },
        },
    });
    let info = doc.add_object(dictionary! { "Author" => Object::string_literal("Jane Roe") });
    doc.trailer.set("Root", catalog);
    doc.trailer.set("Info", info);
    save(&mut doc)
}

/// The stream bytes of every image XObject in `pdf`, to assert on them.
///
/// # Panics
///
/// Panics if `pdf` is not a readable PDF.
#[must_use]
pub fn images(pdf: &[u8]) -> Vec<Bytes> {
    streams(pdf, |dict| dict_name(dict, b"Subtype") == Some(b"Image"))
}

/// The stream bytes of every embedded file in `pdf`, to assert on them.
///
/// # Panics
///
/// Panics if `pdf` is not a readable PDF.
#[must_use]
pub fn attachments(pdf: &[u8]) -> Vec<Bytes> {
    streams(pdf, |dict| {
        dict_name(dict, b"Type") == Some(b"EmbeddedFile")
    })
}

/// Whether any page of `pdf` still draws an image, as XObject or inline.
///
/// An XObject (`Do`) or an inline image (`BI`).
///
/// # Panics
///
/// Panics if `pdf` is not a readable PDF or a page's content cannot be
/// decoded.
#[must_use]
pub fn draws_images(pdf: &[u8]) -> bool {
    let doc = Document::load_mem(pdf).expect("a readable PDF");
    doc.get_pages().into_values().any(|page| {
        let content = Content::decode(&doc.get_page_content(page)).expect("page content");
        content
            .operations
            .iter()
            .any(|operation| matches!(operation.operator.as_str(), "Do" | "BI"))
    })
}

/// A one-page PDF that draws each of `images` (named `Im1`, `Im2`, …), then
/// runs the `extra` operations.
fn drawing(images: Vec<Stream>, extra: Vec<Operation>) -> Bytes {
    let mut doc = Document::with_version("1.5");
    let pages = doc.new_object_id();
    let mut xobjects = Dictionary::new();
    let mut operations = Vec::new();
    for (index, image) in images.into_iter().enumerate() {
        let id = doc.add_object(image);
        let name = format!("Im{}", index + 1);
        xobjects.set(name.as_bytes(), id);
        operations.push(Operation::new("Do", vec![Object::Name(name.into_bytes())]));
    }
    operations.extend(extra);
    let resources = doc.add_object(dictionary! { "XObject" => xobjects });
    let content = Content { operations }
        .encode()
        .expect("encode page content");
    let contents = doc.add_object(Stream::new(dictionary! {}, content));
    let page = doc.add_object(dictionary! {
        "Type" => "Page", "Parent" => pages, "Contents" => contents, "Resources" => resources,
    });
    doc.objects.insert(
        pages,
        Object::Dictionary(dictionary! {
            "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1,
            "MediaBox" => vec![0.into(), 0.into(), 100.into(), 100.into()],
        }),
    );
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages });
    doc.trailer.set("Root", catalog);
    save(&mut doc)
}

/// An image XObject dictionary of `width` x `height` 8-bit RGB samples, stored
/// with `filter` (or none).
fn image_dict(width: i64, height: i64, filter: Option<&str>) -> Dictionary {
    let mut dict = dictionary! {
        "Type" => "XObject",
        "Subtype" => "Image",
        "Width" => width,
        "Height" => height,
        "ColorSpace" => "DeviceRGB",
        "BitsPerComponent" => 8,
    };
    if let Some(filter) = filter {
        dict.set("Filter", Object::Name(filter.as_bytes().to_vec()));
    }
    dict
}

/// An 8x8 white JPEG file.
fn jpeg() -> Vec<u8> {
    let image = RgbImage::from_pixel(8, 8, Rgb([255, 255, 255]));
    let mut out = Cursor::new(Vec::new());
    DynamicImage::ImageRgb8(image)
        .write_to(&mut out, ImageFormat::Jpeg)
        .expect("encode JPEG fixture");
    out.into_inner()
}

/// The bytes of every stream in `pdf` whose dictionary `matches`.
fn streams(pdf: &[u8], matches: impl Fn(&Dictionary) -> bool) -> Vec<Bytes> {
    let doc = Document::load_mem(pdf).expect("a readable PDF");
    doc.objects
        .values()
        .filter_map(|object| match object {
            Object::Stream(stream) if matches(&stream.dict) => {
                Some(Bytes::from(stream.content.clone()))
            }
            _ => None,
        })
        .collect()
}

/// The name `dict` holds under `key`.
fn dict_name<'a>(dict: &'a Dictionary, key: &[u8]) -> Option<&'a [u8]> {
    dict.get(key).and_then(Object::as_name).ok()
}

/// `doc` serialized.
fn save(doc: &mut Document) -> Bytes {
    let mut out = Vec::new();
    doc.save_to(&mut out).expect("save PDF fixture");
    Bytes::from(out)
}
