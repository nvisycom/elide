//! Hidden content through the codec: stripped at decode and checked on
//! encode.

use bytes::Bytes;
use elide_codec::DocumentPart;
use elide_codec::content::ContentData;
use elide_core::redaction::{Disposition, HiddenContentPolicy};
use lopdf::{Document, Object, Stream, dictionary};

mod common;

use common::{format, streams};

/// A one-page PDF with an `/Info` author and one attached `notes.txt`.
fn pdf() -> Vec<u8> {
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
            "EmbeddedFiles" => dictionary! { "Names" => vec![Object::string_literal("notes.txt"), spec.into()] },
        },
    });
    let info = doc.add_object(dictionary! { "Author" => Object::string_literal("Jane Roe") });
    doc.trailer.set("Root", catalog);
    doc.trailer.set("Info", info);
    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    out
}

async fn decode(policy: HiddenContentPolicy) -> elide_codec::Document {
    format()
        .decode(ContentData::new(Bytes::from(pdf())), policy)
        .await
        .expect("decode")
}

#[tokio::test]
async fn decode_removes_metadata_and_surfaces_attachments() {
    let doc = decode(HiddenContentPolicy::default()).await;
    assert!(doc.changed_at_decode());
    let attachment = doc.parts().iter().find_map(|part| match part {
        DocumentPart::Blob { id, bytes, hint } if id.as_str().starts_with("att-") => {
            Some((bytes.clone(), hint.clone()))
        }
        _ => None,
    });
    assert_eq!(
        attachment,
        Some((Bytes::from_static(b"Jane Roe"), "txt".to_owned()))
    );
    let out = doc.encode().expect("encode");
    assert!(!contains(out.as_bytes(), b"/Author"), "Info survived");
}

#[tokio::test]
async fn redacted_metadata_is_still_stripped() {
    let policy = HiddenContentPolicy::default().with_metadata(Disposition::Redact);
    let doc = decode(policy).await;
    let out = doc.encode().expect("encode");
    assert!(!contains(out.as_bytes(), b"/Author"), "Info survived");
}

#[tokio::test]
async fn a_removed_attachment_is_dropped_on_encode() {
    let mut doc = decode(HiddenContentPolicy::default()).await;
    let id = doc
        .parts()
        .iter()
        .find_map(|part| match part {
            DocumentPart::Blob { id, .. } if id.as_str().starts_with("att-") => Some(id.clone()),
            _ => None,
        })
        .expect("attachment blob");
    doc.remove_part(&id).unwrap();
    let out = doc.encode().expect("encode");
    assert!(
        streams(out.as_bytes(), b"EmbeddedFile").is_empty(),
        "attachment survived"
    );
}

/// Whether `haystack` contains `needle`.
fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}
