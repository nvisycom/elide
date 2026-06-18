#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]

//! Codec traits and format handlers for reading and redacting documents.
//!
//! A codec turns raw bytes into a streamable, redactable
//! [`DocumentHandle`] and back. The [`CodecRegistry`] resolves an
//! extension or content type to a [`Format`], whose [`Loader`] decodes
//! bytes into a [`Handler`] for some [`Modality`].
//!
//! Modality erasure is *open*: a decoded handle is returned as an
//! [`UntypedDocumentHandle`] and recovered to the concrete
//! [`DocumentHandle<M>`] with [`UntypedDocumentHandle::into`], a `TypeId`
//! downcast that works for any modality — built-in or custom — with no
//! central enum of kinds.
//!
//! [`Modality`]: elide_core::modality::Modality

mod codec;
pub mod content;
pub mod handler;

pub use self::codec::{
    CodecRegistry, DocumentHandle, Format, FormatId, Handler, Loader, UntypedDocumentHandle,
};

#[cfg(all(test, feature = "txt"))]
mod tests {
    use elide_core::modality::DataWriter;
    use elide_core::modality::text::{Text, TextLocation, TextReplacement};
    use elide_core::redaction::Redactions;

    use super::*;

    #[tokio::test]
    async fn registry_decodes_txt_by_extension() {
        let reg = CodecRegistry::with_builtin();
        let handle = reg
            .decode("hello\nworld\n", "txt")
            .await
            .expect("txt decoded");
        assert_eq!(handle.format_id().as_str(), "elide.text.txt");
        assert!(handle.is::<Text>());
    }

    #[tokio::test]
    async fn decode_content_resolves_from_filename() {
        use content::ContentData;

        let reg = CodecRegistry::with_builtin();
        let content = ContentData::from_text("hello\nworld\n").with_filename("notes.txt");
        let handle = reg
            .decode_content(content)
            .await
            .expect("resolved by filename");
        assert_eq!(handle.format_id().as_str(), "elide.text.txt");
    }

    #[tokio::test]
    async fn decode_content_resolves_from_content_type() {
        use content::ContentData;

        let reg = CodecRegistry::with_builtin();
        let content = ContentData::from_text("plain").with_content_type("text/plain");
        let handle = reg
            .decode_content(content)
            .await
            .expect("resolved by content type");
        assert_eq!(handle.format_id().as_str(), "elide.text.txt");
    }

    #[tokio::test]
    async fn decode_content_without_hints_is_an_error() {
        use content::ContentData;

        let reg = CodecRegistry::with_builtin();
        assert!(
            reg.decode_content(ContentData::from_text("x"))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn untyped_into_wrong_modality_returns_self() {
        let reg = CodecRegistry::with_builtin();
        let handle = reg.decode("hi", "txt").await.expect("decoded");
        // Recover as Text succeeds; the TypeId downcast is exact.
        let typed = handle.into::<Text>().expect("text handle");
        assert_eq!(typed.format_id().as_str(), "elide.text.txt");
    }

    #[tokio::test]
    async fn decode_redact_reencode_round_trip() {
        let reg = CodecRegistry::with_builtin();
        let handle = reg
            .decode("contact alice@example.test today", "txt")
            .await
            .expect("decoded");
        let mut doc = handle.into::<Text>().expect("text handle");

        let mut batch = Redactions::new();
        // "contact " is 8 bytes; "alice@example.test" is 18 → 8..26.
        batch.push(
            TextLocation::new(8, 26),
            TextReplacement::substituted("[EMAIL]"),
        );
        doc.write_at(batch).await.expect("redacted");

        let out = doc.encode().expect("re-encoded");
        assert_eq!(out.as_bytes(), b"contact [EMAIL] today");
    }

    #[tokio::test]
    async fn unknown_extension_is_an_error() {
        let reg = CodecRegistry::with_builtin();
        assert!(reg.decode("data", "xyz").await.is_err());
    }
}
