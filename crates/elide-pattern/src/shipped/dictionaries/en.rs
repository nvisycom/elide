//! English-language dictionaries, terms written in English
//! and meant to fire on English documents.
//!
//! Demonyms, religion names, and language names translate
//! across locales (`American` → `américain`, `Catholic` →
//! `catolico`, `English` → `englisch`). The terms here are the
//! English forms; a non-English document needs its own locale
//! sub-module (`fr`, `es`, …) once those land. Runtime
//! filtering by the call's language keeps these
//! dictionaries from firing when the caller asserts a
//! non-English document.

use crate::{__shipped_dictionary as shipped_dictionary, Dictionary};

shipped_dictionary!(
    /// Long-form English fiat currency names (US Dollar, Euro,
    /// Polish Zloty, …). English-scoped, so the prose names don't
    /// fire on French / German documents. The language-agnostic
    /// ISO 4217 codes live in `world::currency_codes`.
    fn currencies
        from "../../../assets/dictionaries/en/finance/currencies.toml"
        with csv "../../../assets/dictionaries/en/finance/currencies.csv"
);
shipped_dictionary!(
    /// English names of human languages plus ISO 639-1 codes
    /// (English, en, French, fr, …).
    fn languages
        from "../../../assets/dictionaries/en/personal/languages.toml"
        with csv "../../../assets/dictionaries/en/personal/languages.csv"
);
shipped_dictionary!(
    /// English demonyms and nationality terms (American, French,
    /// Japanese, …).
    fn nationalities
        from "../../../assets/dictionaries/en/personal/nationalities.toml"
        with text "../../../assets/dictionaries/en/personal/nationalities.txt"
);
shipped_dictionary!(
    /// English names of religious affiliations (Christian, Muslim,
    /// Buddhist, …).
    fn religions
        from "../../../assets/dictionaries/en/personal/religions.toml"
        with text "../../../assets/dictionaries/en/personal/religions.txt"
);

/// Every English-scoped built-in dictionary.
#[must_use]
pub fn all() -> Vec<Dictionary> {
    vec![currencies(), languages(), nationalities(), religions()]
}
