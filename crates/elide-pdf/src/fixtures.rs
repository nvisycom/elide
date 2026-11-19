//! Small in-memory PDF fixtures, behind the `fixtures` feature.
//!
//! A downstream crate testing the PDF codec needs real documents carrying
//! embedded images, attachments, and metadata, and a way to check what an
//! encoded document still holds, without taking a direct dependency on a PDF
//! or image crate. These helpers build and inspect such fixtures so the test
//! stays in this crate's vocabulary.

use std::io::Cursor;

use ::image::{DynamicImage, ImageFormat, Rgb, RgbImage};
use bytes::Bytes;
use hayro_syntax::object::{Object as ReadObject, Stream as ReadStream};

use crate::pdf::Pdf;
use crate::pdf::content::{Operation, encode, parse};
use crate::pdf::object::{Dictionary, Id, Stream, Value, dict};
use crate::pdf::reader::{DictExt, StreamExt, page_content};
use crate::pdf::writer::{Document, compressed};

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
    let image = compressed(image_dict(2, 2, None), &[200; 12]).expect("compress raw samples");
    drawing(vec![image], Vec::new())
}

/// A one-page scanned PDF: no text, one 2x2 image of RGB `(200, 200, 200)`
/// drawn over the whole 100x100pt page.
#[must_use]
pub fn scanned_image() -> Bytes {
    let image = compressed(image_dict(2, 2, None), &[200; 12]).expect("compress raw samples");
    page_of(vec![image], Vec::new(), Some(100))
}

/// A one-page PDF drawing three images, two the image pipeline cannot read.
///
/// A fax-coded image (`CCITTFaxDecode`), a JPEG image, and an inline image; the
/// JPEG is the one it can read.
#[must_use]
pub fn unsurfaceable_images() -> Bytes {
    let fax = Stream::new(image_dict(8, 8, Some("CCITTFaxDecode")), vec![0; 4]);
    let jpeg = Stream::new(image_dict(8, 8, Some("DCTDecode")), jpeg());
    let inline = b"BI /W 1 /H 1 /BPC 8 /CS /G ID \x00 EI".to_vec();
    drawing(vec![fax, jpeg], inline)
}

/// A one-page PDF with an author in `/Info` and one attached file.
///
/// Its `/Info` names its author, `Jane Roe`, and it attaches one file,
/// `notes.txt`, holding `Jane Roe`.
#[must_use]
pub fn metadata_and_attachment() -> Bytes {
    let mut doc = Document::new((1, 7));
    let file = doc.add(Stream::new(
        dict! { "Type" => "EmbeddedFile" },
        b"Jane Roe".to_vec(),
    ));
    let spec = doc.add(dict! {
        "Type" => "Filespec",
        "F" => Value::string("notes.txt"),
        "EF" => dict! { "F" => file },
    });
    with_pages(&mut doc, [Dictionary::new()]);
    catalog(&mut doc).set(
        "Names",
        dict! {
            "EmbeddedFiles" => dict! {
                "Names" => vec![Value::string("notes.txt"), spec.into()],
            },
        },
    );
    let info = doc.add(dict! { "Author" => Value::string("Jane Roe") });
    doc.set_info(info);
    save(&doc)
}

/// The stream bytes of every image XObject in `pdf`, to assert on them.
///
/// # Panics
///
/// Panics if `pdf` is not a readable PDF.
#[must_use]
pub fn images(pdf: &[u8]) -> Vec<Bytes> {
    streams(pdf, |stream| stream.dict().is_subtype(b"Image"))
        .into_iter()
        .map(|(raw, _)| raw)
        .collect()
}

/// The decoded samples of every image in `pdf`, to assert on its pixels.
///
/// # Panics
///
/// Panics if `pdf` is not a readable PDF or an image cannot be decoded.
#[must_use]
pub fn image_samples(pdf: &[u8]) -> Vec<Bytes> {
    streams(pdf, |stream| stream.dict().is_subtype(b"Image"))
        .into_iter()
        .map(|(raw, plain)| plain.unwrap_or(raw))
        .collect()
}

/// The content of every embedded file in `pdf`, decoded, to assert on it.
///
/// # Panics
///
/// Panics if `pdf` is not a readable PDF.
#[must_use]
pub fn attachments(pdf: &[u8]) -> Vec<Bytes> {
    streams(pdf, |stream| stream.dict().is_type(b"EmbeddedFile"))
        .into_iter()
        .map(|(raw, plain)| plain.unwrap_or(raw))
        .collect()
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
    let pdf = Pdf::open(pdf).expect("a readable PDF");
    pdf.pages().iter().any(|page| {
        parse(&page_content(page).expect("page content"))
            .iter()
            .any(|operation| matches!(operation.operator.as_str(), "Do" | "BI"))
    })
}

/// A one-page PDF that draws each of `images` (named `Im1`, `Im2`, …), then
/// runs the `extra` content.
fn drawing(images: Vec<Stream>, extra: Vec<u8>) -> Bytes {
    page_of(images, extra, None)
}

/// A one-page PDF that draws each of `images`, scaled to `size` points square
/// when given, then runs the `extra` content.
fn page_of(images: Vec<Stream>, extra: Vec<u8>, size: Option<i64>) -> Bytes {
    let mut doc = Document::new((1, 5));
    let mut xobjects = Dictionary::new();
    let mut operations = Vec::new();
    for (index, image) in images.into_iter().enumerate() {
        let id = doc.add(image);
        let name = format!("Im{}", index + 1);
        xobjects.set(&name, id);
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
        operations.push(Operation::new("Do", vec![Value::name(&name)]));
        if size.is_some() {
            operations.push(Operation::new("Q", vec![]));
        }
    }
    let mut content = encode(&operations).expect("encode page content");
    content.push(b'\n');
    content.extend(extra);
    let contents = doc.add(Stream::new(Dictionary::new(), content));
    with_pages(
        &mut doc,
        [dict! {
            "Contents" => contents,
            "Resources" => dict! { "XObject" => xobjects },
            "MediaBox" => vec![0.into(), 0.into(), 100.into(), 100.into()],
        }],
    );
    save(&doc)
}

/// An image XObject dictionary of `width` x `height` 8-bit RGB samples, stored
/// with `filter` (or none).
fn image_dict(width: i64, height: i64, filter: Option<&str>) -> Dictionary {
    let mut dict = dict! {
        "Type" => "XObject",
        "Subtype" => "Image",
        "Width" => width,
        "Height" => height,
        "ColorSpace" => "DeviceRGB",
        "BitsPerComponent" => 8,
    };
    if let Some(filter) = filter {
        dict.set("Filter", filter);
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

/// The bytes of every stream in `pdf` that `matches`: as stored, and as
/// decoded when its filters decode.
fn streams(pdf: &[u8], matches: impl Fn(&ReadStream<'_>) -> bool) -> Vec<(Bytes, Option<Bytes>)> {
    let pdf = Pdf::open(pdf).expect("a readable PDF");
    pdf.file()
        .objects()
        .into_iter()
        .filter_map(|object| match object {
            ReadObject::Stream(stream) if matches(&stream) => Some((
                Bytes::from(stream.raw_data().into_owned()),
                stream.plain().ok().map(Bytes::from),
            )),
            _ => None,
        })
        .collect()
}

/// A one-page PDF attaching `files`, each `(name, bytes)`, each under its
/// own kid of the `/EmbeddedFiles` tree.
#[cfg(test)]
pub(crate) fn with_attachments(files: &[(&str, &[u8])]) -> Bytes {
    save(&attachments_document(files))
}

/// The document [`with_attachments`] writes.
#[cfg(test)]
pub(crate) fn attachments_document(files: &[(&str, &[u8])]) -> Document {
    let mut doc = Document::new((1, 7));
    let mut kids = Vec::new();
    for (name, bytes) in files {
        let stream = compressed(
            dict! {
                "Type" => "EmbeddedFile",
                "Params" => dict! { "Size" => bytes.len() },
            },
            bytes,
        )
        .expect("compress an attached file");
        let stream = doc.add(stream);
        let spec = doc.add(dict! {
            "Type" => "Filespec",
            "F" => Value::string(name),
            "EF" => dict! { "F" => stream },
        });
        let kid = doc.add(dict! { "Names" => vec![Value::string(name), spec.into()] });
        kids.push(Value::Ref(kid));
    }
    with_pages(&mut doc, [Dictionary::new()]);
    catalog(&mut doc).set(
        "Names",
        dict! { "EmbeddedFiles" => dict! { "Kids" => kids } },
    );
    doc
}

/// Give `doc` a page tree of `pages`, each made a `/Page` under it, and a
/// catalog over the tree; each page's id, in order.
pub(crate) fn with_pages(
    doc: &mut Document,
    pages: impl IntoIterator<Item = Dictionary>,
) -> Vec<Id> {
    let tree = doc.reserve();
    let ids: Vec<Id> = pages
        .into_iter()
        .map(|mut page| {
            page.set("Type", "Page");
            page.set("Parent", tree);
            doc.add(page)
        })
        .collect();
    let kids: Vec<Value> = ids.iter().map(|&id| id.into()).collect();
    let count = kids.len();
    doc.set(
        tree,
        dict! { "Type" => "Pages", "Kids" => kids, "Count" => count },
    );
    let catalog = doc.add(dict! { "Type" => "Catalog", "Pages" => tree });
    doc.set_root(catalog);
    ids
}

/// The catalog of `doc`, to change.
///
/// # Panics
///
/// Panics if `doc` has no catalog yet.
pub(crate) fn catalog(doc: &mut Document) -> &mut Dictionary {
    let root = doc.root().expect("a catalog");
    doc.get_mut(root)
        .and_then(Value::as_dict_mut)
        .expect("a catalog dictionary")
}

/// `doc` serialized.
pub(crate) fn save(doc: &Document) -> Bytes {
    Bytes::from(doc.save().expect("save PDF fixture"))
}

/// `doc` saved and opened.
#[cfg(test)]
pub(crate) fn open(doc: &Document) -> Pdf {
    Pdf::open(&save(doc)).expect("open PDF fixture")
}

/// The object `id` of a document written, as it is read back.
#[cfg(test)]
pub(crate) fn read_id(id: Id) -> crate::pdf::reader::ObjectId {
    crate::pdf::reader::ObjectId::new(id.0, 0)
}

/// The object `id` of `pdf`, owned, its references kept.
#[cfg(test)]
pub(crate) fn object(pdf: &Pdf, id: crate::pdf::reader::ObjectId) -> Value {
    crate::pdf::writer::object_value(&pdf.object(id).expect("an object"))
}

/// `doc` opened with each of `objects` kept: named by its catalog (made
/// with no pages when it has none), so a document written holds them.
#[cfg(test)]
pub(crate) fn held(mut doc: Document, objects: &[Id]) -> Pdf {
    if doc.root().is_none() {
        with_pages(&mut doc, []);
    }
    let held: Vec<Value> = objects.iter().map(|&id| id.into()).collect();
    catalog(&mut doc).set("Held", held);
    open(&doc)
}

/// `value` with each stream it holds in place written as an object of its
/// own in `doc`, as a document holds streams.
#[cfg(test)]
pub(crate) fn objects_of(doc: &mut Document, value: Value) -> Value {
    match value {
        Value::Stream(mut stream) => {
            let dict = std::mem::take(&mut stream.dict);
            let Value::Dict(dict) = objects_of(doc, Value::Dict(dict)) else {
                unreachable!("a dictionary stays one");
            };
            stream.dict = dict;
            Value::Ref(doc.add(*stream))
        }
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .map(|item| objects_of(doc, item))
                .collect(),
        ),
        Value::Dict(dict) => Value::Dict(
            dict.iter()
                .map(|(key, value)| (key.to_vec(), objects_of(doc, value.clone())))
                .collect(),
        ),
        value => value,
    }
}
