//! [`Truncate`]: physically remove the middle of a value, keeping a
//! prefix and/or suffix. Distinct from [`Mask`], which preserves length.
//!
//! [`Mask`]: crate::operators::Mask

use elide_core::entity::Entity;
#[cfg(feature = "tabular")]
use elide_core::modality::tabular::{Tabular, TabularReplacement};
use elide_core::modality::text::{Text, TextData, TextReplacement};
use elide_core::operator::{LeakProfile, Operator, OperatorId};
use elide_core::{Error, ErrorKind, Result};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Physically truncate the value, keeping a leading and/or trailing run.
///
/// Unlike [`Mask`], which overwrites the middle with a mask character and
/// keeps the original length, [`Truncate`] *shortens* the string: the
/// dropped characters leave no placeholder. This is the shape PCI DSS
/// v4.0.1 §3.5.1 requires for a stored PAN rendered unreadable by
/// truncation — `"411111"` (BIN only), not `"4111********1234"`.
///
/// Counts are character-based, so multi-byte codepoints stay intact.
/// A configuration whose kept regions already cover (or overlap) the
/// whole value is rejected at apply time: truncating there would either
/// be a no-op or reveal the entire value, so an explicit error beats a
/// silent pass-through.
///
/// [`Mask`]: crate::operators::Mask
#[derive(Debug, Clone, Copy, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Truncate {
    keep_prefix: usize,
    keep_suffix: usize,
}

impl Truncate {
    /// Identity shared by every modality's impl.
    fn id() -> OperatorId {
        OperatorId::new("truncate", "1.0.0")
    }

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

    /// Truncate `value`: the first `keep_prefix` characters concatenated
    /// with the last `keep_suffix`. Errors when the kept regions cover the
    /// whole value (the truncation would be a no-op or reveal everything).
    fn render(&self, value: &str) -> Result<String> {
        let chars: Vec<char> = value.chars().collect();
        let total = chars.len();
        if self.keep_prefix.saturating_add(self.keep_suffix) >= total {
            return Err(Error::new(
                ErrorKind::Redaction,
                "truncate keeps at least the whole value; nothing would be dropped",
            ));
        }
        let suffix_start = total - self.keep_suffix;
        let mut out = String::with_capacity(self.keep_prefix + self.keep_suffix);
        out.extend(chars[..self.keep_prefix].iter());
        out.extend(chars[suffix_start..].iter());
        Ok(out)
    }
}

#[async_trait::async_trait]
impl Operator<Text> for Truncate {
    fn id(&self) -> OperatorId {
        Truncate::id()
    }

    fn leak_profile(&self) -> LeakProfile {
        // The dropped middle is gone irrecoverably; the kept ends leak
        // as-is. Partial captures "some observable shape survives".
        LeakProfile::Partial
    }

    async fn anonymize(&self, _entity: &Entity<Text>, data: &TextData) -> Result<TextReplacement> {
        Ok(TextReplacement::substituted(self.render(data.as_str())?))
    }
}

#[cfg(feature = "tabular")]
#[async_trait::async_trait]
impl Operator<Tabular> for Truncate {
    fn id(&self) -> OperatorId {
        Truncate::id()
    }

    fn leak_profile(&self) -> LeakProfile {
        LeakProfile::Partial
    }

    async fn anonymize(
        &self,
        _entity: &Entity<Tabular>,
        data: &TextData,
    ) -> Result<TabularReplacement> {
        Ok(TextReplacement::substituted(self.render(data.as_str())?).into())
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

    #[test]
    fn errors_when_config_would_be_a_no_op() {
        // keep_prefix + keep_suffix == len: nothing dropped.
        assert!(Truncate::new(4, 4).render("12345678").is_err());
        // keep_prefix + keep_suffix > len: would reveal everything.
        assert!(Truncate::new(6, 6).render("1234567").is_err());
    }

    #[test]
    fn is_character_based_not_byte_based() {
        // Four 2-byte characters; keep 1 + 1, drop the middle two.
        assert_eq!(Truncate::new(1, 1).render("áéíó").unwrap(), "áó");
    }
}
