//! Text redaction (glyph deletion): deleting the glyphs of detected spans from
//! a composite (Type0/Identity-H, CID) subset font, the case a naive
//! text-operator rewrite corrupts, and the recursive document-sanitise pass
//! that deletes structures retaining copies of text, including objects nested
//! below an annotation.

use elide_pdf::Pdf;
use elide_pdf::redact::Detection;

/// A one-page PDF whose text is drawn with an embedded TrueType **subset** in a
/// Type0/Identity-H (CID) font, the encoding that defeats whole-operand text
/// rewriting. It reads `"Contact alice@example.com at the office."` then
/// `"Keep this line intact."`.
const CID_FONT: &[u8] = include_bytes!("testdata/cid_font.pdf");

fn extracted(pdf_bytes: &[u8]) -> String {
    Pdf::open(pdf_bytes)
        .unwrap()
        .extract()
        .blocks
        .iter()
        .map(|b| b.text.to_string())
        .collect()
}

/// Char spans on any page whose text equals `needle`.
fn spans_for(pdf: &Pdf, needle: &str) -> Vec<Detection> {
    let mut out = Vec::new();
    for (page, text) in pdf.page_texts().unwrap() {
        let mut from = 0;
        while let Some(pos) = text[from..].find(needle) {
            let byte_at = from + pos;
            let start = text[..byte_at].chars().count();
            out.push(Detection::new(page, start, start + needle.chars().count()));
            from = byte_at + needle.len();
        }
    }
    out
}

#[test]
fn deletes_a_detected_span_in_a_cid_subset_font() {
    let pdf = Pdf::open(CID_FONT).unwrap();
    let detections = spans_for(&pdf, "alice@example.com");
    assert!(
        !detections.is_empty(),
        "the fixture should carry the target"
    );

    let out = pdf.redact_text(&detections).unwrap();
    let text = extracted(&out);

    // The target is gone; the surrounding text (same CID font) is intact and
    // uncorrupted, deletion never re-encodes, so no glyph is mangled.
    assert!(!text.contains("alice@example.com"), "target survived");
    assert!(text.contains("Contact"), "leading context lost");
    assert!(text.contains("office"), "trailing context lost");
    assert!(text.contains("Keep this line intact"), "second line lost");
}

#[test]
fn sanitize_deletes_a_nested_annotation_subtree() {
    use lopdf::content::{Content, Operation};
    use lopdf::{Document, Object, Stream, dictionary};

    // A page with a link annotation whose appearance stream (/AP -> /N)
    // references a further object holding secret text, a grandchild of the
    // annotation. A one-level GC would leave that grandchild orphaned.
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();

    let font_id = doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Courier",
    });
    let resources_id = doc.add_object(dictionary! { "Font" => dictionary! { "F1" => font_id } });
    let body = Content {
        operations: vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 12.into()]),
            Operation::new("Td", vec![72.into(), 700.into()]),
            Operation::new("Tj", vec![Object::string_literal("Visible body")]),
            Operation::new("ET", vec![]),
        ],
    };
    let content_id = doc.add_object(Stream::new(dictionary! {}, body.encode().unwrap()));

    // Grandchild: a nested resource holding a secret string.
    let nested_id = doc.add_object(dictionary! {
        "Secret" => Object::string_literal("GRANDCHILD-SECRET"),
    });
    // The annotation's appearance stream references the grandchild.
    let ap_stream = Stream::new(
        dictionary! { "Type" => "XObject", "Subtype" => "Form", "Nested" => nested_id },
        b"q Q".to_vec(),
    );
    let ap_id = doc.add_object(ap_stream);
    let annot_id = doc.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Link",
        "Rect" => vec![0.into(), 0.into(), 10.into(), 10.into()],
        "AP" => dictionary! { "N" => Object::Reference(ap_id) },
    });
    let page_id = doc.add_object(dictionary! {
        "Type" => "Page", "Parent" => pages_id,
        "Contents" => content_id, "Resources" => resources_id,
        "Annots" => vec![annot_id.into()],
    });
    let pages = dictionary! {
        "Type" => "Pages", "Kids" => vec![page_id.into()], "Count" => 1,
        "MediaBox" => vec![0.into(), 0.into(), 200.into(), 200.into()],
    };
    doc.objects.insert(pages_id, Object::Dictionary(pages));
    let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog_id);
    let mut pdf = Vec::new();
    doc.save_to(&mut pdf).unwrap();

    // Sanity: the grandchild secret is in the source.
    assert!(String::from_utf8_lossy(&pdf).contains("GRANDCHILD-SECRET"));

    // With no detections, sanitize still runs; isolate its behaviour.
    let out = Pdf::open(&pdf).unwrap().redact_text(&[]).unwrap();

    // The whole annotation subtree, including the grandchild, is gone.
    assert!(
        !String::from_utf8_lossy(&out).contains("GRANDCHILD-SECRET"),
        "nested annotation object survived sanitize"
    );
    // The visible body survives.
    assert!(extracted(&out).contains("Visible body"));
}
