//! End-to-end reversibility test: encrypt entities in a document with the
//! [`Anonymizer`], then recover the originals with the [`Deanonymizer`].
#![cfg(feature = "aes")]

use elide::redaction::operators::AesEncrypt;
use elide::redaction::{Anonymizer, Deanonymizer};
use elide_core::Result;
use elide_core::entity::provenance::{Event, PatternEvent, Provenance};
use elide_core::entity::{Entity, LabelRef};
use elide_core::modality::text::{Text, TextData, TextLocation, TextReplacement};
use elide_core::modality::{DataReader, DataWriter};
use elide_core::operator::Redactions;
use elide_core::primitive::Confidence;

/// An in-memory read/write text document: reads byte ranges and applies a
/// batch of substitutions, right-to-left so earlier offsets stay valid.
struct TextDoc(String);

impl DataReader<Text> for TextDoc {
    async fn read_at(&self, location: &TextLocation) -> Result<Option<TextData>> {
        Ok(self.0.get(location.start..location.end).map(TextData::new))
    }
}

impl DataWriter<Text> for TextDoc {
    async fn write_at(&mut self, mut redactions: Redactions<Text>) -> Result<()> {
        redactions.sort_by_position();
        for (location, replacement) in redactions.iter().rev() {
            let value = match replacement {
                TextReplacement::Substituted(s) => s.as_str(),
                TextReplacement::Removed => "",
            };
            self.0.replace_range(location.start..location.end, value);
        }
        Ok(())
    }
}

fn entity(label: &str, start: usize, end: usize) -> Entity<Text> {
    let location = TextLocation::new(start, end);
    let event = Event::pattern(
        "t",
        Confidence::MAX,
        location.clone(),
        PatternEvent::default(),
    );
    Entity::new(
        LabelRef::new(label),
        location,
        Confidence::MAX,
        Provenance::new(event),
    )
}

#[tokio::test]
async fn encrypt_then_decrypt_recovers_the_original_document() {
    let key = [42u8; 32];

    //                            0         1         2
    //                            0123456789012345678901234
    let mut doc = TextDoc("email a@b.com now".to_string());
    // "a@b.com" occupies bytes 6..13.
    let mut email = entity("EMAIL_ADDRESS", 6, 13);

    // AesEncrypt under the label.
    Anonymizer::<Text>::new()
        .with_label(LabelRef::new("EMAIL_ADDRESS"), AesEncrypt::new(key))
        .anonymize(&mut doc, std::slice::from_mut(&mut email))
        .await
        .unwrap();

    // The original is gone; a ciphertext now sits in its place.
    assert!(!doc.0.contains("a@b.com"));
    let ciphertext_len = doc.0.len() - "email  now".len();
    assert!(ciphertext_len > 0);

    // The entity's location now spans the ciphertext that replaced it.
    let start = "email ".len();
    let encrypted = entity("EMAIL_ADDRESS", start, start + ciphertext_len);

    // Decrypt under the same label.
    Deanonymizer::<Text>::new()
        .with_label(LabelRef::new("EMAIL_ADDRESS"), AesEncrypt::new(key))
        .deanonymize(&mut doc, std::slice::from_ref(&encrypted))
        .await
        .unwrap();

    assert_eq!(doc.0, "email a@b.com now");
}

#[tokio::test]
async fn wrong_key_leaves_the_ciphertext_in_place() {
    let mut doc = TextDoc("x secret y".to_string());
    let mut secret = entity("TOKEN", 2, 8);

    Anonymizer::<Text>::new()
        .with_label(LabelRef::new("TOKEN"), AesEncrypt::new([1u8; 32]))
        .anonymize(&mut doc, std::slice::from_mut(&mut secret))
        .await
        .unwrap();
    let encrypted_doc = doc.0.clone();
    let ct_len = doc.0.len() - "x  y".len();
    let encrypted = entity("TOKEN", 2, 2 + ct_len);

    // A deanonymizer with the wrong key cannot recover, so it skips the
    // entity and leaves the ciphertext untouched.
    Deanonymizer::<Text>::new()
        .with_label(LabelRef::new("TOKEN"), AesEncrypt::new([2u8; 32]))
        .deanonymize(&mut doc, std::slice::from_ref(&encrypted))
        .await
        .unwrap();

    assert_eq!(doc.0, encrypted_doc);
}
