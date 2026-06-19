//! [`Pseudonymize`]: replace an entity with a consistent synthetic value,
//! backed by a vault so every mention of one entity reads the same.

use elide_core::Result;
use elide_core::entity::{Entity, LabelRef};
use elide_core::modality::TextBacked;
use elide_core::modality::text::{TextData, TextReplacement};
use elide_core::redaction::{LeakProfile, Operator, OperatorId, Vault};

use crate::redaction::generator::Generator;

/// Replace an entity with a consistent generated surrogate.
///
/// The pseudonymizing operator: where [`Replace`] writes a fixed marker
/// and [`Hash`] a digest, `Pseudonymize` substitutes a per-entity
/// surrogate from a [`Generator`] and — crucially — replays the *same*
/// surrogate every time that entity recurs. That consistency is what
/// preserves a document's referential structure: "Alice told Bob, then
/// Alice left" keeps two distinct, stable stand-ins for Alice and Bob
/// rather than collapsing or scattering them. The default [`RandomToken`]
/// generator mints opaque tokens; swap in one that produces believable
/// names when readability matters.
///
/// # How consistency is keyed
///
/// Each entity reduces to a key of `(label, seed)`, where the seed is the
/// entity's [coreference] id when it has one, else its original value.
/// Coreference is the stronger signal: it ties `Alice`, `she`, and
/// `Ms. Smith` to one cluster, so all three draw the same surrogate even
/// though their surface text differs. Absent coreference, equal values
/// still collapse to one surrogate.
///
/// The key resolves through the vault with [`get_or_try_insert_with`], so
/// the first mention generates and stores the surrogate and every later
/// mention reads it back — atomically, even if mentions are pseudonymized
/// concurrently. Folding the `label` into the key lets one vault back
/// several `Pseudonymize` operators without their surrogates colliding.
///
/// # Recoverability
///
/// [`LeakProfile::Recoverable`]: the mapping from surrogate back to the
/// original lives in the vault. Whoever holds the vault can reverse it;
/// without it the surrogate is just an opaque token.
///
/// [`Vault`]: elide_core::redaction::Vault
/// [`Generator`]: crate::redaction::generator::Generator
/// [`RandomToken`]: crate::redaction::generator::RandomToken
/// [`get_or_try_insert_with`]: elide_core::redaction::Vault::get_or_try_insert_with
/// [coreference]: elide_core::entity::EntityCoRef
/// [`Replace`]: super::Replace
/// [`Hash`]: super::Hash
#[derive(Debug, Clone)]
pub struct Pseudonymize<V, G> {
    vault: V,
    generator: G,
}

impl<V, G> Pseudonymize<V, G> {
    /// A pseudonymizer that mints surrogates with `generator` and keeps
    /// them consistent in `vault`.
    pub fn new(vault: V, generator: G) -> Self {
        Self { vault, generator }
    }
}

/// The vault key for an entity: its label paired with the cluster seed
/// (coreference id when present, else the original value).
type Key = (LabelRef, String);

impl<M, V, G> Operator<M> for Pseudonymize<V, G>
where
    M: TextBacked,
    V: Vault<Key, TextReplacement>,
    G: Generator<M>,
{
    fn id(&self) -> OperatorId {
        OperatorId::new("pseudonymize", "1.0.0")
    }

    fn leak_profile(&self) -> LeakProfile {
        // The surrogate-to-original mapping is recoverable to whoever
        // holds the vault.
        LeakProfile::Recoverable
    }

    async fn anonymize(&self, entity: &Entity<M>, data: &TextData) -> Result<TextReplacement> {
        let seed = entity.coref.as_ref().map_or_else(
            || data.as_str().to_owned(),
            |coref| coref.as_str().to_owned(),
        );
        let key = (entity.label.clone(), seed.clone());

        self.vault
            .get_or_try_insert_with(key, || Ok(self.generator.generate(&entity.label, &seed)))
            .await
    }
}

#[cfg(test)]
mod tests {
    use elide_core::entity::provenance::{Event, PatternEvent, Provenance};
    use elide_core::entity::{EntityCoRef, LabelRef};
    use elide_core::modality::text::{Text, TextLocation};
    use elide_core::primitive::Confidence;

    use super::*;
    use crate::redaction::InMemoryVault;
    use crate::redaction::generator::RandomToken;

    fn entity(label: &str, coref: Option<&str>) -> Entity<Text> {
        let location = TextLocation::new(0, 1);
        let event = Event::pattern(
            "t",
            Confidence::MAX,
            location.clone(),
            PatternEvent::default(),
        );
        let entity = Entity::new(
            LabelRef::new(label),
            location,
            Confidence::MAX,
            Provenance::new(event),
        );
        match coref {
            Some(c) => entity.with_coref(EntityCoRef::new(c.to_owned())),
            None => entity,
        }
    }

    #[tokio::test]
    async fn coreferent_mentions_share_one_surrogate() {
        let op = Pseudonymize::new(InMemoryVault::new(), RandomToken);
        // "Alice" and "she" are one cluster; their surface text differs.
        let alice = entity("PERSON", Some("c1"));
        let she = entity("PERSON", Some("c1"));

        let a = op.anonymize(&alice, &TextData::new("Alice")).await.unwrap();
        let b = op.anonymize(&she, &TextData::new("she")).await.unwrap();

        assert_eq!(a, b); // same cluster -> same surrogate
    }

    #[tokio::test]
    async fn distinct_clusters_get_distinct_surrogates() {
        let op = Pseudonymize::new(InMemoryVault::new(), RandomToken);
        let alice = entity("PERSON", Some("c1"));
        let bob = entity("PERSON", Some("c2"));

        let a = op.anonymize(&alice, &TextData::new("Alice")).await.unwrap();
        let b = op.anonymize(&bob, &TextData::new("Bob")).await.unwrap();

        assert_ne!(a, b);
    }

    #[tokio::test]
    async fn cloned_operator_shares_the_vault() {
        // Cloning the operator clones the vault's Arc, so both halves draw
        // from one map: the same cluster reads one surrogate across them.
        let op = Pseudonymize::new(InMemoryVault::new(), RandomToken);
        let clone = op.clone();
        let entity = entity("PERSON", Some("c1"));

        let a = op
            .anonymize(&entity, &TextData::new("Alice"))
            .await
            .unwrap();
        let b = clone
            .anonymize(&entity, &TextData::new("Alice"))
            .await
            .unwrap();

        assert_eq!(a, b);
    }

    #[tokio::test]
    async fn without_coref_equal_values_share_a_surrogate() {
        let op = Pseudonymize::new(InMemoryVault::new(), RandomToken);
        let first = entity("PERSON", None);
        let second = entity("PERSON", None);

        let a = op.anonymize(&first, &TextData::new("Alice")).await.unwrap();
        let b = op
            .anonymize(&second, &TextData::new("Alice"))
            .await
            .unwrap();

        assert_eq!(a, b); // same value -> same surrogate
    }
}
