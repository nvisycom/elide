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
use lopdf::{Dictionary, Document, Object, ObjectId, Stream, dictionary};

use crate::document::object::DictionaryExt;

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

/// A one-page scanned PDF: no text, one 2x2 image of RGB `(200, 200, 200)`
/// drawn over the whole 100x100pt page.
#[must_use]
pub fn scanned_image() -> Bytes {
    let mut image = Stream::new(image_dict(2, 2, None), vec![200; 12]);
    image.compress().expect("compress raw samples");
    drawing_scaled(vec![image], 100)
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
    with_pages(&mut doc, [Dictionary::new()]);
    let file = doc.add_object(Stream::new(
        dictionary! { "Type" => "EmbeddedFile" },
        b"Jane Roe".to_vec(),
    ));
    let spec = doc.add_object(dictionary! {
        "Type" => "Filespec",
        "F" => Object::string_literal("notes.txt"),
        "EF" => dictionary! { "F" => file },
    });
    doc.catalog_mut().expect("a catalog").set(
        "Names",
        dictionary! {
            "EmbeddedFiles" => dictionary! {
                "Names" => vec![Object::string_literal("notes.txt"), spec.into()],
            },
        },
    );
    let info = doc.add_object(dictionary! { "Author" => Object::string_literal("Jane Roe") });
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
    streams(pdf, |doc, dict| dict.is_subtype(doc, b"Image"))
}

/// The decoded samples of every image in `pdf`, to assert on its pixels.
///
/// # Panics
///
/// Panics if `pdf` is not a readable PDF or an image cannot be decoded.
#[must_use]
pub fn image_samples(pdf: &[u8]) -> Vec<Bytes> {
    let doc = Document::load_mem(pdf).expect("a readable PDF");
    doc.objects
        .values()
        .filter_map(|object| match object {
            Object::Stream(stream) if stream.dict.is_subtype(&doc, b"Image") => {
                let samples = stream
                    .decompressed_content()
                    .unwrap_or_else(|_| stream.content.clone());
                Some(Bytes::from(samples))
            }
            _ => None,
        })
        .collect()
}

/// The stream bytes of every embedded file in `pdf`, to assert on them.
///
/// # Panics
///
/// Panics if `pdf` is not a readable PDF.
#[must_use]
pub fn attachments(pdf: &[u8]) -> Vec<Bytes> {
    streams(pdf, |doc, dict| dict.is_type(doc, b"EmbeddedFile"))
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
    page_of(images, extra, None)
}

/// A one-page PDF that draws each of `images` scaled to `size` points square.
fn drawing_scaled(images: Vec<Stream>, size: i64) -> Bytes {
    page_of(images, Vec::new(), Some(size))
}

/// A one-page PDF that draws each of `images`, scaled to `size` points square
/// when given, then runs the `extra` operations.
fn page_of(images: Vec<Stream>, extra: Vec<Operation>, size: Option<i64>) -> Bytes {
    let mut doc = Document::with_version("1.5");
    let mut xobjects = Dictionary::new();
    let mut operations = Vec::new();
    for (index, image) in images.into_iter().enumerate() {
        let id = doc.add_object(image);
        let name = format!("Im{}", index + 1);
        xobjects.set(name.as_bytes(), id);
        if let Some(size) = size {
            operations.push(Operation::new("q", vec![]));
            operations.push(Operation::new(
                "cm",
                vec![
                    size.into(),
                    0.into(),
                    0.into(),
                    size.into(),
                    0.into(),
                    0.into(),
                ],
            ));
        }
        operations.push(Operation::new("Do", vec![Object::Name(name.into_bytes())]));
        if size.is_some() {
            operations.push(Operation::new("Q", vec![]));
        }
    }
    operations.extend(extra);
    let resources = doc.add_object(dictionary! { "XObject" => xobjects });
    let content = Content { operations }
        .encode()
        .expect("encode page content");
    let contents = doc.add_object(Stream::new(dictionary! {}, content));
    with_pages(
        &mut doc,
        [dictionary! {
            "Contents" => contents, "Resources" => resources,
            "MediaBox" => vec![0.into(), 0.into(), 100.into(), 100.into()],
        }],
    );
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
fn streams(pdf: &[u8], matches: impl Fn(&Document, &Dictionary) -> bool) -> Vec<Bytes> {
    let doc = Document::load_mem(pdf).expect("a readable PDF");
    doc.objects
        .values()
        .filter_map(|object| match object {
            Object::Stream(stream) if matches(&doc, &stream.dict) => {
                Some(Bytes::from(stream.content.clone()))
            }
            _ => None,
        })
        .collect()
}

/// A one-page PDF attaching `files`, each `(name, bytes)`, each under its
/// own kid of the `/EmbeddedFiles` tree.
#[cfg(test)]
pub(crate) fn with_attachments(files: &[(&str, &[u8])]) -> Bytes {
    let mut doc = Document::with_version("1.7");
    with_pages(&mut doc, [Dictionary::new()]);
    let mut kids = Vec::new();
    for (name, bytes) in files {
        let mut stream = Stream::new(
            dictionary! {
                "Type" => "EmbeddedFile",
                "Params" => dictionary! { "Size" => bytes.len() as i64 },
            },
            bytes.to_vec(),
        );
        stream.compress().expect("compress an attached file");
        let stream = doc.add_object(stream);
        let spec = doc.add_object(dictionary! {
            "Type" => "Filespec",
            "F" => Object::string_literal(*name),
            "EF" => dictionary! { "F" => stream },
        });
        let kid = doc.add_object(dictionary! {
            "Names" => vec![Object::string_literal(*name), spec.into()],
        });
        kids.push(Object::Reference(kid));
    }
    doc.catalog_mut().expect("a catalog").set(
        "Names",
        dictionary! { "EmbeddedFiles" => dictionary! { "Kids" => kids } },
    );
    save(&mut doc)
}

/// Give `doc` a page tree of `pages`, each made a `/Page` under it, and a
/// catalog over the tree; each page's id, in order.
pub(crate) fn with_pages(
    doc: &mut Document,
    pages: impl IntoIterator<Item = Dictionary>,
) -> Vec<ObjectId> {
    let tree = doc.new_object_id();
    let ids: Vec<ObjectId> = pages
        .into_iter()
        .map(|mut page| {
            page.set("Type", "Page");
            page.set("Parent", tree);
            doc.add_object(page)
        })
        .collect();
    let kids: Vec<Object> = ids.iter().map(|&id| id.into()).collect();
    let count = kids.len() as i64;
    doc.objects.insert(
        tree,
        Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => count }),
    );
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => tree });
    doc.trailer.set("Root", catalog);
    ids
}

/// `doc` serialized.
fn save(doc: &mut Document) -> Bytes {
    let mut out = Vec::new();
    doc.save_to(&mut out).expect("save PDF fixture");
    Bytes::from(out)
}
