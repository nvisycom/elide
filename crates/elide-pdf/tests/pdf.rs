//! Extract + rewrite over a minimal born-digital PDF built with lopdf.

use elide_pdf::block::Replacement;
use elide_pdf::{ErrorKind, Pdf};
use lopdf::content::{Content, Operation};
use lopdf::{Document, Object, Stream, dictionary};

/// A one-page PDF whose content stream draws `text` with a Courier font, so
/// the text layer is extractable and rewritable.
fn text_pdf(text: &str) -> Vec<u8> {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let font_id = doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Courier",
    });
    let resources_id = doc.add_object(dictionary! {
        "Font" => dictionary! { "F1" => font_id },
    });
    let content = Content {
        operations: vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 48.into()]),
            Operation::new("Td", vec![100.into(), 600.into()]),
            Operation::new("Tj", vec![Object::string_literal(text)]),
            Operation::new("ET", vec![]),
        ],
    };
    let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
    let page_id = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "Contents" => content_id,
    });
    let pages = dictionary! {
        "Type" => "Pages",
        "Kids" => vec![page_id.into()],
        "Count" => 1,
        "Resources" => resources_id,
        "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
    };
    doc.objects.insert(pages_id, Object::Dictionary(pages));
    let catalog_id = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    doc.trailer.set("Root", catalog_id);

    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    out
}

#[test]
fn extracts_page_text() {
    let pdf = text_pdf("Alice");
    let extraction = Pdf::open(&pdf).unwrap().extract();
    assert!(
        extraction
            .blocks
            .iter()
            .any(|b| b.page == 1 && b.text.contains("Alice")),
        "blocks: {:?}",
        extraction.blocks
    );
}

#[test]
fn a_text_page_has_no_issues() {
    let pdf = text_pdf("Alice");
    let extraction = Pdf::open(&pdf).unwrap().extract();
    // The page yielded text, so there is nothing to flag.
    assert!(extraction.issues.is_empty());
}

#[test]
fn a_textless_page_is_flagged_needs_ocr() {
    use elide_pdf::block::IssueKind;

    // A one-page PDF whose content stream draws no text.
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let content = Content { operations: vec![] };
    let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
    let page_id = doc.add_object(dictionary! {
        "Type" => "Page", "Parent" => pages_id, "Contents" => content_id,
    });
    let pages = dictionary! {
        "Type" => "Pages", "Kids" => vec![page_id.into()], "Count" => 1,
        "MediaBox" => vec![0.into(), 0.into(), 100.into(), 100.into()],
    };
    doc.objects.insert(pages_id, Object::Dictionary(pages));
    let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog_id);
    let mut pdf = Vec::new();
    doc.save_to(&mut pdf).unwrap();

    let extraction = Pdf::open(&pdf).unwrap().extract();
    assert!(extraction.blocks.is_empty());
    assert_eq!(extraction.issues.len(), 1);
    assert_eq!(extraction.issues[0].page, 1);
    assert_eq!(extraction.issues[0].kind, IssueKind::NeedsOcr);
}

#[test]
fn rewrite_replaces_born_digital_text() {
    let pdf = text_pdf("Alice");
    let out = Pdf::open(&pdf)
        .unwrap()
        .rewrite(&[Replacement::new(1, "Alice", "[NAME]")])
        .unwrap();

    // Re-extract the rewritten document: the original is gone, replacement present.
    let after = Pdf::open(&out).unwrap().extract();
    let text: String = after.blocks.iter().map(|b| b.text.as_str()).collect();
    assert!(text.contains("[NAME]"), "not redacted: {text}");
    assert!(!text.contains("Alice"), "original survived: {text}");
}

#[test]
fn empty_rewrite_resaves_unchanged() {
    let pdf = text_pdf("Alice");
    let out = Pdf::open(&pdf).unwrap().rewrite(&[]).unwrap();
    let after = Pdf::open(&out).unwrap().extract();
    assert!(after.blocks.iter().any(|b| b.text.contains("Alice")));
}

#[test]
fn rewrite_is_fail_closed_when_text_absent() {
    let pdf = text_pdf("Alice");
    let err = Pdf::open(&pdf)
        .unwrap()
        .rewrite(&[Replacement::new(1, "Bob", "[NAME]")])
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::UnsafeRewrite);
}

#[test]
fn not_a_pdf_is_invalid_document() {
    let err = Pdf::open(b"this is not a pdf").unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidDocument);
}

/// A one-page PDF carrying `text` and one embedded image XObject (a tiny raw
/// RGB image), so embedding extraction and replacement can be exercised.
///
/// Returns the document bytes, the image XObject's id, and the page's content
/// stream id (a stream that is *not* an image, for fail-closed tests).
fn image_pdf(text: &str, image: &[u8]) -> (Vec<u8>, (u32, u16), (u32, u16)) {
    use lopdf::Dictionary;

    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let font_id = doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Courier",
    });

    // A minimal image XObject: 2x2 8-bit RGB, no filter (raw samples).
    let mut img_dict = Dictionary::new();
    img_dict.set("Type", Object::Name(b"XObject".to_vec()));
    img_dict.set("Subtype", Object::Name(b"Image".to_vec()));
    img_dict.set("Width", Object::Integer(2));
    img_dict.set("Height", Object::Integer(2));
    img_dict.set("ColorSpace", Object::Name(b"DeviceRGB".to_vec()));
    img_dict.set("BitsPerComponent", Object::Integer(8));
    let image_id = doc.add_object(Stream::new(img_dict, image.to_vec()));

    let resources_id = doc.add_object(dictionary! {
        "Font" => dictionary! { "F1" => font_id },
        "XObject" => dictionary! { "Im1" => image_id },
    });
    let content = Content {
        operations: vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 48.into()]),
            Operation::new("Td", vec![100.into(), 600.into()]),
            Operation::new("Tj", vec![Object::string_literal(text)]),
            Operation::new("ET", vec![]),
            Operation::new("Do", vec![Object::Name(b"Im1".to_vec())]),
        ],
    };
    let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
    let page_id = doc.add_object(dictionary! {
        "Type" => "Page", "Parent" => pages_id,
        "Contents" => content_id, "Resources" => resources_id,
    });
    let pages = dictionary! {
        "Type" => "Pages", "Kids" => vec![page_id.into()], "Count" => 1,
        "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
    };
    doc.objects.insert(pages_id, Object::Dictionary(pages));
    let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog_id);

    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    (out, image_id, content_id)
}

#[test]
fn extracts_embedded_images() {
    let pixels = vec![255u8; 2 * 2 * 3];
    let (pdf, image_id, _) = image_pdf("Alice", &pixels);
    let extraction = Pdf::open(&pdf).unwrap().extract();

    assert_eq!(extraction.embeddings.len(), 1);
    let emb = &extraction.embeddings[0];
    assert_eq!((emb.id.number, emb.id.generation), image_id);
    assert_eq!(emb.width, 2);
    assert_eq!(emb.height, 2);
    assert_eq!(emb.kind, elide_pdf::image::EmbeddingKind::Raw);
    assert_eq!(emb.bytes.as_ref(), pixels.as_slice());
}

#[test]
fn rewrite_with_images_replaces_stream_content() {
    let pixels = vec![255u8; 2 * 2 * 3];
    let (pdf, _, _) = image_pdf("Alice", &pixels);
    let extraction = Pdf::open(&pdf).unwrap().extract();
    let id = extraction.embeddings[0].id;

    let blanked = vec![0u8; 2 * 2 * 3];
    let out = Pdf::open(&pdf)
        .unwrap()
        .rewrite_with_images(
            &[],
            &[elide_pdf::image::ImageReplacement {
                id,
                bytes: blanked.clone(),
            }],
        )
        .unwrap();

    let after = Pdf::open(&out).unwrap().extract();
    assert_eq!(after.embeddings[0].bytes.as_ref(), blanked.as_slice());
    assert_ne!(after.embeddings[0].bytes.as_ref(), pixels.as_slice());
}

#[test]
fn rewrite_with_images_is_fail_closed_on_unknown_id() {
    let (pdf, _, _) = image_pdf("Alice", &[255u8; 12]);
    let err = Pdf::open(&pdf)
        .unwrap()
        .rewrite_with_images(
            &[],
            &[elide_pdf::image::ImageReplacement {
                id: elide_pdf::image::ImageId::new(9999, 0),
                bytes: vec![0u8; 12],
            }],
        )
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::UnsafeRewrite);
}

#[test]
fn rewrite_with_images_is_fail_closed_on_non_image_stream() {
    // Target the page's content stream — a stream, but not an image XObject.
    let (pdf, _, content_id) = image_pdf("Alice", &[255u8; 12]);
    let err = Pdf::open(&pdf)
        .unwrap()
        .rewrite_with_images(
            &[],
            &[elide_pdf::image::ImageReplacement {
                id: elide_pdf::image::ImageId::new(content_id.0, content_id.1),
                bytes: vec![0u8; 12],
            }],
        )
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::UnsafeRewrite);
}

#[test]
fn a_shared_image_is_surfaced_once() {
    use lopdf::Dictionary;

    // A two-page PDF where both pages reference the same image XObject through
    // one shared Resources dictionary.
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();

    let mut img_dict = Dictionary::new();
    img_dict.set("Type", Object::Name(b"XObject".to_vec()));
    img_dict.set("Subtype", Object::Name(b"Image".to_vec()));
    img_dict.set("Width", Object::Integer(2));
    img_dict.set("Height", Object::Integer(2));
    img_dict.set("ColorSpace", Object::Name(b"DeviceRGB".to_vec()));
    img_dict.set("BitsPerComponent", Object::Integer(8));
    let image_id = doc.add_object(Stream::new(img_dict, vec![255u8; 2 * 2 * 3]));

    let resources_id = doc.add_object(dictionary! {
        "XObject" => dictionary! { "Im1" => image_id },
    });
    let draw = Content {
        operations: vec![Operation::new("Do", vec![Object::Name(b"Im1".to_vec())])],
    };
    let mut page = |content: &Content| {
        let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
        doc.add_object(dictionary! {
            "Type" => "Page", "Parent" => pages_id,
            "Contents" => content_id, "Resources" => resources_id,
        })
    };
    let page1 = page(&draw);
    let page2 = page(&draw);
    let pages = dictionary! {
        "Type" => "Pages", "Kids" => vec![page1.into(), page2.into()], "Count" => 2,
        "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
    };
    doc.objects.insert(pages_id, Object::Dictionary(pages));
    let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog_id);
    let mut pdf = Vec::new();
    doc.save_to(&mut pdf).unwrap();

    let extraction = Pdf::open(&pdf).unwrap().extract();
    // The same image on two pages is surfaced once, on its first page.
    assert_eq!(extraction.embeddings.len(), 1);
    assert_eq!(
        (
            extraction.embeddings[0].id.number,
            extraction.embeddings[0].id.generation
        ),
        image_id
    );
    assert_eq!(extraction.embeddings[0].page, 1);
}
