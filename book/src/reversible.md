# Reversible redaction

Some redactions must be undone later by someone entitled to: a support
agent who needs the customer's real email, a researcher re-identifying a
record with approval. Elide has two ways to keep that possible.

## Pseudonymization

`Pseudonymize` replaces each entity with a generated surrogate, and uses the
same surrogate every time that entity appears. "Alice told Bob, then Alice
left" keeps two distinct, stable stand-ins, so the redacted text still reads
coherently. When coreference is known, "Alice", "she", and "Ms. Smith" share
one surrogate.

```rust
use elide::prelude::operators::*;
use elide::prelude::*;
use elide::redaction::generator::RandomToken;
use elide::redaction::vault::InMemoryVault;

let anonymizer = Anonymizer::<Text>::new()
    .with(Rule::label(
        builtins::PERSON_NAME.to_ref(),
        Pseudonymize::new(InMemoryVault::new(), RandomToken),
    ));
```

The vault maps surrogates to originals. Whoever holds it can reverse the
substitution. Share one vault across documents to keep surrogates consistent
between them, and implement `Vault` to keep it somewhere durable.

## Encryption

`AesEncrypt` (feature `operator-aes`) replaces each entity with its
AES-256-GCM ciphertext. A `Deanonymizer` with the same key decrypts it in
place:

```rust
use elide::codec::DocumentPart;
use elide::modality::text::TextLocation;
use elide::prelude::operators::*;
use elide::prelude::*;

# #[tokio::main]
# async fn main() -> Result<()> {
# let mut document = FormatRegistry::with_builtin().decode("mail a@b.com now", "txt").await?;
# let DocumentPart::Stream { handle, .. } = &mut document.parts_mut()[0] else { unreachable!() };
# let stream = handle.downcast_mut::<Text>().unwrap();
# let scope = Scope::new().with_catalog(LabelCatalog::with_builtins());
# let mut entities = Analyzer::<Text>::new()
#     .with_recognizer(elide::recognition::pattern::PatternRecognizer::builder().with_builtin_patterns().build()?)
#     .analyze_stream(stream, &scope).await?.entities;
# assert_eq!(entities.len(), 1);
let key = vec![42u8; 32];

Anonymizer::<Text>::new()
    .with(Rule::label(builtins::EMAIL_ADDRESS.to_ref(), AesEncrypt::with_key(key.clone())))
    .anonymize(stream, &mut entities, &scope)
    .await?;
# let redacted = String::from_utf8(document.encode()?.as_bytes().to_vec()).unwrap();
# assert!(!redacted.contains("a@b.com"));
# let ciphertext = "mail ".len()..redacted.len() - " now".len();
# let DocumentPart::Stream { handle, .. } = &mut document.parts_mut()[0] else { unreachable!() };
# let stream = handle.downcast_mut::<Text>().unwrap();

// Later, by whoever holds the key: an entity for each ciphertext in the
// redacted document, under the label it was encrypted with.
let encrypted: Entity<Text> =
    Entity::custom(builtins::EMAIL_ADDRESS.to_ref(), TextLocation::inline(ciphertext)).into();

Deanonymizer::<Text>::new()
    .with_label(builtins::EMAIL_ADDRESS.to_ref(), AesEncrypt::with_key(key))
    .deanonymize(stream, &[encrypted])
    .await?;
# assert_eq!(document.encode()?.as_bytes(), b"mail a@b.com now");
# Ok(())
# }
```

The deanonymizer works on the redacted document, so its entities must point
at the ciphertext, not at the original values: the anonymizer's entities
keep the spans of the originals. Locate the ciphertext the way your
application stores redacted documents, for example by recording where each
replacement landed.

A deanonymizer with the wrong key cannot recover a value, so it leaves the
ciphertext where it is. Keep the key out of the code: `KeyProvider` supplies
it from wherever keys live.
