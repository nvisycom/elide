//! End-to-end reversibility test: encrypt entities in a document with the
//! [`Anonymizer`], then recover the originals with the [`Deanonymizer`].

use elide_core::entity::LabelRef;
use elide_core::modality::text::Text;
use elide_core::recognition::Scope;
use elide_operator::operators::AesEncrypt;
use elide_redaction::{Anonymizer, Deanonymizer, Rule};

mod fixtures;
use fixtures::{TextDoc, entity};

#[tokio::test]
async fn encrypt_then_decrypt_recovers_the_original_document() {
    let key = [42u8; 32];

    //                    0         1         2
    //                    0123456789012345678901234
    let mut doc = TextDoc::new("email a@b.com now");
    // "a@b.com" occupies bytes 6..13.
    let mut email = entity("EMAIL_ADDRESS", (6, 13));

    // AesEncrypt under the label.
    Anonymizer::<Text>::new()
        .with(Rule::label(
            LabelRef::new("EMAIL_ADDRESS"),
            AesEncrypt::with_key(key.to_vec()),
        ))
        .anonymize(
            &mut doc,
            std::slice::from_mut(&mut email),
            &Scope::default(),
        )
        .await
        .unwrap();

    // The original is gone; a ciphertext now sits in its place.
    assert!(!doc.text().contains("a@b.com"));
    let ciphertext_len = doc.text().len() - "email  now".len();
    assert!(ciphertext_len > 0);

    // The entity's location now spans the ciphertext that replaced it.
    let start = "email ".len();
    let encrypted = entity("EMAIL_ADDRESS", (start, start + ciphertext_len));

    // Decrypt under the same label.
    Deanonymizer::<Text>::new()
        .with_label(
            LabelRef::new("EMAIL_ADDRESS"),
            AesEncrypt::with_key(key.to_vec()),
        )
        .deanonymize(&mut doc, std::slice::from_ref(&encrypted))
        .await
        .unwrap();

    assert_eq!(doc.text(), "email a@b.com now");
}

#[tokio::test]
async fn wrong_key_leaves_the_ciphertext_in_place() {
    let mut doc = TextDoc::new("x secret y");
    let mut secret = entity("TOKEN", (2, 8));

    Anonymizer::<Text>::new()
        .with(Rule::label(
            LabelRef::new("TOKEN"),
            AesEncrypt::with_key([1u8; 32].to_vec()),
        ))
        .anonymize(
            &mut doc,
            std::slice::from_mut(&mut secret),
            &Scope::default(),
        )
        .await
        .unwrap();
    let encrypted_doc = doc.text().to_owned();
    let ct_len = doc.text().len() - "x  y".len();
    let encrypted = entity("TOKEN", (2, 2 + ct_len));

    // A deanonymizer with the wrong key cannot recover, so it skips the entity
    // and leaves the ciphertext untouched.
    Deanonymizer::<Text>::new()
        .with_label(
            LabelRef::new("TOKEN"),
            AesEncrypt::with_key([2u8; 32].to_vec()),
        )
        .deanonymize(&mut doc, std::slice::from_ref(&encrypted))
        .await
        .unwrap();

    assert_eq!(doc.text(), encrypted_doc);
}
