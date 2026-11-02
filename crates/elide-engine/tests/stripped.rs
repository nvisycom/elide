//! The report records the hidden content decode strips: an embedded file no
//! codec can read is reported before redaction removes it, and the applied
//! report records it afresh even when built by hand.

use bytes::Bytes;
use elide_codec::mocks::{MOCK_EXT, MockPart, encode_mock, mock_format};
use elide_core::Result;
use elide_core::redaction::{HiddenContent, HiddenContentPolicy};
use elide_engine::{Directives, Document, Orchestrator, PartId, Report};
use elide_format::FormatRegistry;

/// A mock container whose one blob carries a hint no codec is registered for.
fn container() -> Bytes {
    encode_mock(
        "body",
        &[MockPart {
            id: "notes.bin".to_owned(),
            hint: "bin".to_owned(),
            bytes: Bytes::from_static(b"unreadable notes"),
        }],
    )
}

#[tokio::test]
async fn an_unreadable_embedding_is_reported_before_it_is_removed() -> Result<()> {
    let registry = FormatRegistry::new().with_format(mock_format());
    let orchestrator = Orchestrator::new().with_registry(registry.clone());
    let unreadable = PartId::new("outer.mock").child("notes.bin");

    let handle = registry
        .decode(container(), MOCK_EXT, HiddenContentPolicy::default())
        .await?;
    let mut documents = [Document::new("outer.mock", handle)];

    let analyzed = orchestrator
        .analyze(&mut documents, &Directives::new())
        .await?;
    let reported: Vec<HiddenContent> = analyzed
        .report
        .part_stripped(&unreadable)
        .map(|item| item.content)
        .collect();
    assert_eq!(reported, [HiddenContent::Unreadable]);

    // A report built by hand carries no stripped section; applying records it.
    let applied = orchestrator
        .anonymize_with(&mut documents, Report::new())
        .await?;
    assert_eq!(applied.part_stripped(&unreadable).count(), 1);
    let out = documents[0].document.encode()?;
    assert!(
        !out.as_bytes()
            .windows(b"unreadable notes".len())
            .any(|w| w == b"unreadable notes"),
        "the unreadable embedding shipped"
    );
    Ok(())
}
