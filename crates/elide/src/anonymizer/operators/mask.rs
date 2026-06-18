//! [`Mask`]: replace characters of the matched value with a fixed mask
//! character, optionally leaving a prefix and/or suffix visible.

use elide_core::Error;
use elide_core::entity::Entity;
use elide_core::modality::text::{Text, TextData, TextReplacement};
use elide_core::redaction::{LeakProfile, Operator, OperatorId};

/// Character-replacement masking operator.
///
/// Configurable along three axes: the `mask_char` substituted in
/// (default `'*'`), a `keep_prefix` of leading characters left visible,
/// and a `keep_suffix` of trailing ones. Counts are character-based, so
/// multi-byte codepoints stay intact, and the output length equals the
/// input length. When `keep_prefix + keep_suffix` covers the whole
/// value, it passes through unmasked.
///
/// Common patterns: mask everything ([`Mask::stars`]); show the last 4
/// of a card (`Mask::stars().with_keep_suffix(4)`).
#[derive(Debug, Clone, Copy)]
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
    /// Character-based; when the kept regions cover the whole value it
    /// is returned unchanged.
    pub fn render(&self, value: &str) -> String {
        let chars: Vec<char> = value.chars().collect();
        let total = chars.len();
        if self.keep_prefix.saturating_add(self.keep_suffix) >= total {
            return value.to_owned();
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

impl Operator<Text> for Mask {
    fn id(&self) -> OperatorId {
        OperatorId::new("mask", "1.0.0")
    }

    fn leak_profile(&self) -> LeakProfile {
        // Length, position, and any kept prefix/suffix are observable.
        LeakProfile::Partial
    }

    async fn anonymize(
        &self,
        _entity: &Entity<Text>,
        data: &TextData,
    ) -> Result<TextReplacement, Error> {
        Ok(TextReplacement::substituted(self.render(data.as_str())))
    }
}
