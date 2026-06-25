//! [`Pseudonymize`]: replace an entity with a consistent synthetic value,
//! backed by a vault so every mention of one entity reads the same.

use elide_core::Result;
use elide_core::entity::{Entity, LabelRef};
use elide_core::modality::Modality;
#[cfg(feature = "tabular")]
use elide_core::modality::tabular::{Tabular, TabularReplacement};
use elide_core::modality::text::{Text, TextData, TextReplacement};
use elide_core::operator::{LeakProfile, Operator, OperatorId};
#[cfg(feature = "tabular")]
use elide_core::{Error, ErrorKind};

use crate::generator::Generator;
use crate::vault::Vault;

/// Replace an entity with a consistent generated surrogate.
///
/// The pseudonymizing operator: where [`Replace`] writes a fixed marker
/// and `Sha2Hash` a digest, `Pseudonymize` substitutes a per-entity
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
/// [`Vault`]: crate::vault::Vault
/// [`Generator`]: crate::generator::Generator
/// [`RandomToken`]: crate::generator::RandomToken
/// [`get_or_try_insert_with`]: crate::vault::Vault::get_or_try_insert_with
/// [coreference]: elide_core::entity::EntityCoRef
/// [`Replace`]: super::Replace
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

    /// Identity shared by every modality's impl.
    fn id() -> OperatorId {
        OperatorId::new("pseudonymize", "1.0.0")
    }
}

/// The vault key for an entity: its label paired with the cluster seed
/// (coreference id when present, else the original value).
type Key = (LabelRef, String);

/// The cluster seed for an entity: its coreference id when present, else its
/// original value.
fn seed<M: Modality>(entity: &Entity<M>, data: &TextData) -> String {
    entity.coref.as_ref().map_or_else(
        || data.as_str().to_owned(),
        |coref| coref.as_str().to_owned(),
    )
}

impl<V, G> Operator<Text> for Pseudonymize<V, G>
where
    V: Vault<Key, TextReplacement>,
    G: Generator<Text>,
{
    fn id(&self) -> OperatorId {
        Self::id()
    }

    fn leak_profile(&self) -> LeakProfile {
        // The surrogate-to-original mapping is recoverable to whoever
        // holds the vault.
        LeakProfile::Recoverable
    }

    async fn anonymize(&self, entity: &Entity<Text>, data: &TextData) -> Result<TextReplacement> {
        let seed = seed(entity, data);
        let key = (entity.label.clone(), seed.clone());

        self.vault
            .get_or_try_insert_with(key, || Ok(self.generator.generate(&entity.label, &seed)))
            .await
    }
}

#[cfg(feature = "tabular")]
impl<V, G> Operator<Tabular> for Pseudonymize<V, G>
where
    V: Vault<Key, TextReplacement>,
    G: Generator<Tabular>,
{
    fn id(&self) -> OperatorId {
        Self::id()
    }

    fn leak_profile(&self) -> LeakProfile {
        LeakProfile::Recoverable
    }

    async fn anonymize(
        &self,
        entity: &Entity<Tabular>,
        data: &TextData,
    ) -> Result<TabularReplacement> {
        let seed = seed(entity, data);
        let key = (entity.label.clone(), seed.clone());

        // The vault stores the text surrogate; unwrap the generator's cell
        // treatment to get it (a surrogate generator never drops structure).
        let cell = self
            .vault
            .get_or_try_insert_with(key, || {
                match self.generator.generate(&entity.label, &seed) {
                    TabularReplacement::Cell(replacement) => Ok(replacement),
                    _ => Err(Error::new(
                        ErrorKind::Validation,
                        "pseudonymize generator must produce a cell surrogate",
                    )),
                }
            })
            .await?;
        Ok(TabularReplacement::Cell(cell))
    }
}

#[cfg(test)]
mod tests {
    use elide_core::entity::provenance::{Event, PatternEvent, Provenance};
    use elide_core::entity::{EntityCoRef, LabelRef};
    use elide_core::modality::text::{Text, TextLocation};
    use elide_core::primitive::Confidence;

    use super::*;
    use crate::generator::RandomToken;
    use crate::vault::InMemoryVault;

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
