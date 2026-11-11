//! Embedded images through the codec: surfaced as blobs, redacted, written
//! back, or removed.

use bytes::Bytes;
use elide::codec::{Document, LocalId};
use elide::redaction::{HiddenContent, HiddenContentPolicy};
use elide_image::fixtures::png;
use elide_pdf::fixtures;

use super::decode;

/// The PNG signature every surfaced raw-sample image starts with.
const PNG: &[u8] = b"\x89PNG";

/// The embedded-image blobs (`img-*`) a document surfaces, as `(id, bytes)`.
fn image_blobs(doc: &Document) -> Vec<(LocalId, Bytes)> {
    doc.blobs()
        .iter()
        .filter(|blob| blob.id.as_str().starts_with("img-"))
        .map(|blob| (blob.id.clone(), blob.bytes.clone()))
        .collect()
}

/// Raw samples (a lossless image, or a page this crate flattened) surface
/// as a PNG, and a redaction of one is written back and passes the check.
#[tokio::test]
async fn raw_samples_surface_as_png_and_round_trip() {
    let mut doc = decode(fixtures::raw_image(), HiddenContentPolicy::default()).await;
    let blobs = image_blobs(&doc);
    assert_eq!(blobs.len(), 1);
    assert!(blobs[0].1.starts_with(PNG));

    doc.replace_blob(&blobs[0].0, png(2, 2)).unwrap();
    let encoded = doc.encode().expect("a redacted raw image passes the check");
    assert_eq!(fixtures::images(encoded.as_bytes()).len(), 1);
}

/// What cannot be surfaced (fax samples, an inline image) is removed at
/// decode, and an image no codec could read is removed on encode, its
/// drawing cut rather than blanked.
#[tokio::test]
async fn unsurfaceable_and_unreadable_images_are_removed() {
    let mut doc = decode(
        fixtures::unsurfaceable_images(),
        HiddenContentPolicy::default(),
    )
    .await;
    let unreadable: usize = doc
        .stripped()
        .iter()
        .filter(|entry| entry.content == HiddenContent::Unreadable)
        .map(|entry| entry.count)
        .sum();
    assert_eq!(unreadable, 2);

    let blobs = image_blobs(&doc);
    assert_eq!(blobs.len(), 1);
    doc.remove_blob(&blobs[0].0).unwrap();
    let encoded = doc.encode().expect("encode");
    assert!(fixtures::images(encoded.as_bytes()).is_empty());
    assert!(!fixtures::draws_images(encoded.as_bytes()));
}

#[tokio::test]
async fn document_surfaces_and_replaces_an_image() {
    let source = fixtures::jpeg_image();
    let mut doc = decode(source.clone(), HiddenContentPolicy::default()).await;

    // The document surfaces the one embedded image as a blob.
    let blobs = image_blobs(&doc);
    assert_eq!(blobs.len(), 1);

    // Fold a redacted image back into the document; encode writes it over the
    // original.
    doc.replace_blob(&blobs[0].0, png(2, 2)).unwrap();
    let encoded = doc.encode().unwrap();
    let after = fixtures::images(encoded.as_bytes());
    assert_eq!(after.len(), 1);
    assert_ne!(
        after[0],
        fixtures::images(&source)[0],
        "image was not redacted"
    );
}

#[tokio::test]
async fn replace_blob_rejects_an_unknown_id() {
    let mut doc = decode(fixtures::jpeg_image(), HiddenContentPolicy::default()).await;
    let err = doc
        .replace_blob(&LocalId::new("img-9999-0"), png(2, 2))
        .unwrap_err();
    assert_eq!(err.kind(), elide::ErrorKind::NotFound);
}
