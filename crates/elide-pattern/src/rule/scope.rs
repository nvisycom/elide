//! [`RuleScope`]: the labels a compiled rule emits, and the locales it runs in.

use elide_core::entity::{LabelCatalog, LabelRef};
use elide_core::primitive::{CountryCode, LanguageTag};
use elide_core::recognition::{Context, Languages};

/// A rule's candidate labels and locale scope, shared by its compiled forms.
pub(crate) struct RuleScope {
    /// Candidate labels, most-specific first.
    pub labels: Vec<LabelRef>,
    /// Languages the rule applies to. Empty means "any language".
    pub languages: Vec<LanguageTag>,
    /// Countries the rule applies to. Empty means "any country".
    pub countries: Vec<CountryCode>,
}

impl RuleScope {
    /// The label the rule emits this call, or `None` when its locale scope
    /// excludes the call.
    ///
    /// Locale filtering keys on *asserted* languages and countries only. A
    /// detected language never filters: detection is unreliable on short,
    /// word-poor input (a single data cell like `12345678Z` has no language,
    /// yet resolves to an arbitrary one) and would wrongly suppress a valid
    /// locale-scoped match. When the caller asserts a language or country,
    /// that assertion is authoritative and gates the locale rules.
    pub(crate) fn label(&self, languages: &Languages<'_>, ctx: &Context<'_>) -> Option<&LabelRef> {
        if !languages.asserted_apply_to(&self.languages) || !ctx.applies_to_country(&self.countries)
        {
            return None;
        }
        resolve_label(&self.labels, ctx.catalog())
    }
}

/// The label a rule emits under `catalog`: the most-specific candidate the
/// catalog declares, else the most-specific candidate overall.
///
/// A rule always emits *something* so the reconcile layers see every match and
/// can use a strong-but-out-of-catalog detection (a checksum-validated IBAN) to
/// subsume a weak in-catalog one nested inside it (a loose `drivers_license`
/// prefix). Restricting the output to the catalog is a downstream filter run
/// *after* reconciliation, so suppression evidence is not discarded before it
/// can be used. An empty catalog narrows nothing, so the most-specific
/// candidate wins.
fn resolve_label<'a>(candidates: &'a [LabelRef], catalog: &LabelCatalog) -> Option<&'a LabelRef> {
    candidates
        .iter()
        .find(|l| catalog.contains(l))
        .or_else(|| candidates.first())
}
