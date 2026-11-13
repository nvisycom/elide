//! [`Truncate`]: physically remove the middle of a value, keeping a
//! prefix and/or suffix. Distinct from [`Mask`], which preserves length.
//!
//! [`Mask`]: crate::operators::Mask

use elide_core::entity::Entity;
use elide_core::modality::TextRedactable;
use elide_core::modality::text::TextReplacement;
use elide_core::redaction::{LeakProfile, Operator};
use elide_core::{Component, Result};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::operators::TryOperator;

/// Physically truncate the value, keeping a leading and/or trailing run.
///
/// Unlike [`Mask`], which overwrites the middle with a mask character and
/// keeps the original length, [`Truncate`] *shortens* the string: the
/// dropped characters leave no placeholder. This is the shape PCI DSS
/// v4.0.1 §3.5.1 requires for a stored PAN rendered unreadable by
/// truncation, `"411111"` (BIN only), not `"4111********1234"`.
///
/// Counts are character-based, so multi-byte codepoints stay intact.
///
/// A value the kept regions already cover (or overlap) cannot be truncated
/// without revealing all of it, so [`Truncate`] is a [`TryOperator`] and
/// *declines* it. Used directly as an [`Operator`] it erases a declined
/// value (the safe default); wrap it in [`WithFallback`] to choose a
/// different treatment.
///
/// [`WithFallback`]: crate::operators::WithFallback
///
/// [`Mask`]: crate::operators::Mask
#[derive(Debug, Clone, Copy, Default, Component)]
#[component(name = "truncate", version = "1.0.0")]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Truncate {
    keep_prefix: usize,
    keep_suffix: usize,
}

impl Truncate {
    /// Keep the first `keep_prefix` and last `keep_suffix` characters,
    /// dropping the middle.
    pub fn new(keep_prefix: usize, keep_suffix: usize) -> Self {
        Self {
            keep_prefix,
            keep_suffix,
        }
    }

    /// Keep only the first `n` characters (e.g. a card's 6-digit BIN).
    pub fn prefix(n: usize) -> Self {
        Self::new(n, 0)
    }

    /// Keep only the last `n` characters (e.g. a card's last 4).
    pub fn suffix(n: usize) -> Self {
        Self::new(0, n)
    }

    /// Truncate `value`: the first `keep_prefix` characters followed by the
    /// last `keep_suffix`, or `None` when the kept regions cover the whole
    /// value (the operator declines it).
    pub fn render(&self, value: &str) -> Option<String> {
        let chars: Vec<char> = value.chars().collect();
        let total = chars.len();
        if self.keep_prefix.saturating_add(self.keep_suffix) >= total {
            return None;
        }
        let suffix_start = total - self.keep_suffix;
        let mut out = String::with_capacity(self.keep_prefix + self.keep_suffix);
        out.extend(chars[..self.keep_prefix].iter());
        out.extend(chars[suffix_start..].iter());
        Some(out)
    }
}

#[async_trait::async_trait]
impl<M: TextRedactable> Operator<M> for Truncate {
    fn leak_profile(&self) -> LeakProfile {
        // The dropped middle is gone irrecoverably; the kept ends leak
        // as-is. Partial captures "some observable shape survives".
        LeakProfile::Partial
    }

    async fn anonymize(&self, entity: &Entity<M>, data: &M::Data) -> Result<M::Replacement> {
        // Used on its own, a declined value erases, the safe default.
        Ok(self
            .try_anonymize(entity, data)
            .await?
            .unwrap_or_else(|| M::Replacement::from(TextReplacement::Removed)))
    }
}

#[async_trait::async_trait]
impl<M: TextRedactable> TryOperator<M> for Truncate {
    async fn try_anonymize(
        &self,
        _entity: &Entity<M>,
        data: &M::Data,
    ) -> Result<Option<M::Replacement>> {
        Ok(self
            .render(data.as_ref())
            .map(|text| M::Replacement::from(TextReplacement::substituted(text))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_prefix_and_suffix_dropping_the_middle() {
        let op = Truncate::new(6, 4);
        assert_eq!(op.render("4111111111111234").unwrap(), "4111111234");
    }

    #[test]
    fn prefix_only_keeps_the_bin() {
        assert_eq!(
            Truncate::prefix(6).render("4111111111111234").unwrap(),
            "411111"
        );
    }

    #[test]
    fn suffix_only_keeps_the_tail() {
        assert_eq!(
            Truncate::suffix(4).render("4111111111111234").unwrap(),
            "1234"
        );
    }

    /// Used on its own, a declined value is erased; under `WithFallback` the
    /// fallback decides.
    #[tokio::test]
    async fn a_declined_value_erases_or_falls_back() {
        use elide_core::modality::text::{Text, TextData};

        use crate::operators::{Mask, WithFallback};

        let entity = Entity::<Text>::fixture("PAN", (0, 4));
        let data = TextData::new("1234");
        let alone = Operator::<Text>::anonymize(&Truncate::suffix(4), &entity, &data)
            .await
            .unwrap();
        assert_eq!(alone, TextReplacement::Removed);

        let masked = WithFallback::new(Truncate::suffix(4), Mask::stars());
        let out = Operator::<Text>::anonymize(&masked, &entity, &data)
            .await
            .unwrap();
        assert_eq!(out, TextReplacement::substituted("****"));
    }

    #[test]
    fn declines_a_value_the_kept_regions_cover() {
        // keep_prefix + keep_suffix == len: nothing dropped.
        assert!(Truncate::new(4, 4).render("12345678").is_none());
        // keep_prefix + keep_suffix > len: would reveal everything.
        assert!(Truncate::new(6, 6).render("1234567").is_none());
    }

    #[test]
    fn is_character_based_not_byte_based() {
        // Four 2-byte characters; keep 1 + 1, drop the middle two.
        assert_eq!(Truncate::new(1, 1).render("áéíó").unwrap(), "áó");
    }
}
