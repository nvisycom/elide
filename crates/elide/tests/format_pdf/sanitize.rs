//! Hidden content through the codec: stripped at decode and checked on
//! encode.

use bytes::Bytes;
use elide::codec::Document;
use elide::redaction::{Disposition, HiddenContentPolicy};
use elide_pdf::fixtures;

use super::decode;

/// The attachment blob a document surfaces, as `(bytes, hint)`.
fn attachment(doc: &Document) -> Option<(Bytes, String)> {
    doc.blobs()
        .iter()
        .find(|blob| blob.id.as_str().starts_with("att-"))
        .map(|blob| (blob.bytes.clone(), blob.hint.clone()))
}

/// Whether `haystack` contains `needle`.
fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

#[tokio::test]
async fn decode_removes_metadata_and_surfaces_attachments() {
    let doc = decode(
        fixtures::metadata_and_attachment(),
        HiddenContentPolicy::default(),
    )
    .await;
    assert!(doc.changed_at_decode());
    assert_eq!(
        attachment(&doc),
        Some((Bytes::from_static(b"Jane Roe"), "txt".to_owned()))
    );
    let out = doc.encode().expect("encode");
    assert!(!contains(out.as_bytes(), b"/Author"), "Info survived");
}

#[tokio::test]
async fn redacted_metadata_is_still_stripped() {
    let policy = HiddenContentPolicy::default().with_metadata(Disposition::ScanAsContent);
    let doc = decode(fixtures::metadata_and_attachment(), policy).await;
    let out = doc.encode().expect("encode");
    assert!(!contains(out.as_bytes(), b"/Author"), "Info survived");
}

#[tokio::test]
async fn a_removed_attachment_is_dropped_on_encode() {
    let mut doc = decode(
        fixtures::metadata_and_attachment(),
        HiddenContentPolicy::default(),
    )
    .await;
    let id = doc
        .blobs()
        .iter()
        .find(|blob| blob.id.as_str().starts_with("att-"))
        .map(|blob| blob.id.clone())
        .expect("attachment blob");
    doc.remove_blob(&id).unwrap();
    let out = doc.encode().expect("encode");
    assert!(
        fixtures::attachments(out.as_bytes()).is_empty(),
        "attachment survived"
    );
}
