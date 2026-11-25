//! The shared value-minting core behind both [`Fake`] (an [`Operator`]) and
//! [`FakeGenerator`] (a [`Generator`]): pick a locale, seed a deterministic RNG
//! per entity, and dispatch to the per-label fake generators.
//!
//! Kept free of the operator/generator wrappers so both reuse one path: a
//! coreferent mention seeds the same RNG and so mints the same fake value,
//! independent of which wrapper drove it.
//!
//! [`Fake`]: crate::Fake
//! [`FakeGenerator`]: crate::FakeGenerator
//! [`Operator`]: elide_core::redaction::Operator
//! [`Generator`]: elide_core::redaction::Generator

use std::hash::{DefaultHasher, Hash, Hasher};
use std::str::FromStr;

use elide_core::entity::Entity;
use elide_core::modality::Modality;
use elide_core::primitive::LanguageTag;
use fake::Fake;
use fake::rand::SeedableRng;
use fake::rand::rngs::SmallRng;
use uuid::Builder;

use crate::catalog::Context;
use crate::identity::Identity;
use crate::locale::Locale;

/// The locale to fake in: the entity's BCP-47 `language`, else `default`.
fn locale_for(language: Option<&LanguageTag>, default: &LanguageTag) -> Locale {
    Locale::from_tag(language.unwrap_or(default))
}

/// A deterministic RNG seeded from `seed` (a caller salt) and the entity's
/// [`Identity`] (its coreference id, or its UUID when uncorefed), so coreferent
/// mentions draw the same fake value.
fn rng_for(seed: u64, identity: Identity<'_>) -> SmallRng {
    let mut hasher = DefaultHasher::new();
    seed.hash(&mut hasher);
    identity.hash(&mut hasher);
    SmallRng::seed_from_u64(hasher.finish())
}

/// What every fake value is drawn under: a seed that salts the per-entity RNG,
/// and the locale for an entity that carries no language tag.
#[derive(Debug, Clone)]
pub(crate) struct Draw {
    pub(crate) seed: u64,
    pub(crate) default_language: LanguageTag,
}

impl Default for Draw {
    /// An unsalted RNG, defaulting to English.
    fn default() -> Self {
        Self {
            seed: 0,
            default_language: LanguageTag::from_str("en").expect("en is BCP-47"),
        }
    }
}

impl Draw {
    /// Mint a fake value for `entity`, reading `original` (the source text,
    /// for pattern-preserving structured labels). `None` when the label is
    /// outside the fake-data catalogue, so the caller falls back.
    pub(crate) fn value<M: Modality>(&self, entity: &Entity<M>, original: &str) -> Option<String> {
        let locale = locale_for(entity.language(), &self.default_language);
        let mut rng = rng_for(self.seed, Identity::from(entity));
        Context::new(locale, entity.label().as_str(), original).generate(&mut rng)
    }

    /// A deterministic opaque UUIDv4 token for `entity`, the fallback when a
    /// label has no believable fake. Seeded from the same `(seed, identity)`,
    /// so repeated calls, and coreferent mentions, mint the *same* token,
    /// upholding the consistency guarantee for unsupported labels too.
    pub(crate) fn fallback_token<M: Modality>(&self, entity: &Entity<M>) -> String {
        let mut rng = rng_for(self.seed, Identity::from(entity));
        let mut bytes = [0u8; 16];
        for b in &mut bytes {
            let n: u32 = (0..256u32).fake_with_rng(&mut rng);
            *b = n as u8;
        }
        Builder::from_random_bytes(bytes).into_uuid().to_string()
    }
}
