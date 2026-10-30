//! Validator for organization names anchored on a legal-form suffix.

use std::collections::HashSet;
use std::sync::LazyLock;

/// Words that can lead into a company name without belonging to it ("The",
/// "From", "Der"), lowercase.
static STOP_WORDS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    include_str!("../../assets/gazetteers/legal_forms/stop_words.txt")
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect()
});

/// The lead-in words a legal-form pattern skips ahead of a name and this
/// validator rejects at its start.
pub(crate) fn stop_words() -> impl Iterator<Item = &'static str> {
    STOP_WORDS.iter().copied()
}

/// Return `true` unless the name in front of the legal form starts with a
/// lead-in word: `The AG` is an article and an abbreviation, not a company.
/// Compared case-insensitively.
pub fn legal_form(value: &str) -> bool {
    value
        .split_whitespace()
        .next()
        .is_some_and(|first| !STOP_WORDS.contains(first.to_lowercase().as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_a_name_starting_with_a_lead_in_word() {
        assert!(!legal_form("The AG"));
        assert!(!legal_form("DER KG"));
        assert!(!legal_form("from Acme GmbH"));
    }

    #[test]
    fn accepts_a_name() {
        assert!(legal_form("Acme GmbH"));
        assert!(legal_form("Bank of America Corp."));
        assert!(legal_form("Theta Ltd"));
    }
}
