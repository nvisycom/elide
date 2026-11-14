//! [`Mask`]: replace characters of the matched value with a fixed mask
//! character, optionally leaving a prefix and/or suffix visible.

use elide_core::entity::Entity;
use elide_core::modality::TextRedactable;
use elide_core::modality::text::TextReplacement;
use elide_core::redaction::{LeakProfile, Operator};
use elide_core::{Component, Result};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Masks an entity's text by replacing its characters, like `****1234`.
///
/// Configurable along three axes: the `mask_char` substituted in
/// (default `'*'`), a `keep_prefix` of leading characters left visible,
/// and a `keep_suffix` of trailing ones. Counts are character-based, so
/// multi-byte codepoints stay intact, and the output length equals the
/// input length. When `keep_prefix + keep_suffix` covers the whole
/// value, every character is masked: keeping them would reveal it all.
///
/// Common patterns: mask everything ([`Mask::stars`]); show the last 4
/// of a card (`Mask::stars().with_keep_suffix(4)`).
#[derive(Debug, Clone, Copy, Component)]
#[component(name = "mask", version = "1.0.0")]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Mask {
    mask_char: char,
    keep_prefix: usize,
    keep_suffix: usize,
}

impl Mask {
    /// A mask using `mask_char`, with no preserved prefix or suffix.
    pub fn new(mask_char: char) -> Self {
        Self {
            mask_char,
            keep_prefix: 0,
            keep_suffix: 0,
        }
    }

    /// Mask every character with `'*'`.
    pub fn stars() -> Self {
        Self::new('*')
    }

    /// Leave the first `n` characters of the value unmasked.
    #[must_use]
    pub fn with_keep_prefix(mut self, n: usize) -> Self {
        self.keep_prefix = n;
        self
    }

    /// Leave the last `n` characters of the value unmasked.
    #[must_use]
    pub fn with_keep_suffix(mut self, n: usize) -> Self {
        self.keep_suffix = n;
        self
    }

    /// Render `value`: keep `keep_prefix` leading and `keep_suffix`
    /// trailing characters verbatim, replace the rest with `mask_char`.
    ///
    /// Character-based. When the kept regions cover the whole value, every
    /// character is masked instead.
    pub fn render(&self, value: &str) -> String {
        let chars: Vec<char> = value.chars().collect();
        let total = chars.len();
        if self.keep_prefix.saturating_add(self.keep_suffix) >= total {
            return std::iter::repeat_n(self.mask_char, total).collect();
        }
        let suffix_start = total - self.keep_suffix;
        chars
            .into_iter()
            .enumerate()
            .map(|(i, c)| {
                if i < self.keep_prefix || i >= suffix_start {
                    c
                } else {
                    self.mask_char
                }
            })
            .collect()
    }
}

#[async_trait::async_trait]
impl<M: TextRedactable> Operator<M> for Mask {
    fn leak_profile(&self) -> LeakProfile {
        // Length, position, and any kept prefix/suffix are observable.
        LeakProfile::Partial
    }

    async fn anonymize(
        &self,
        _entity: &Entity<M>,
        data: &M::Data,
    ) -> Result<Option<M::Replacement>> {
        let masked = self.render(data.as_ref());
        Ok(Some(M::Replacement::from(TextReplacement::substituted(
            masked,
        ))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_configured_ends() {
        let mask = Mask::stars().with_keep_suffix(4);
        assert_eq!(mask.render("4111111111111234"), "************1234");
    }

    /// Kept ends that cover the whole value would reveal it, so every
    /// character is masked instead.
    #[test]
    fn masks_everything_when_the_kept_ends_cover_the_value() {
        let mask = Mask::stars().with_keep_suffix(4);
        assert_eq!(mask.render("1234"), "****");
        assert_eq!(mask.render("12"), "**");
    }
}
