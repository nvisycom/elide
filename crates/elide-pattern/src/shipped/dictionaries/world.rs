//! Universal dictionaries, apply regardless of jurisdiction or
//! language.
//!
//! These are brand-name lists whose terms transcend locale.
//! Cryptocurrency names (`Bitcoin`, `Ethereum`) and tickers
//! (`BTC`, `ETH`) are the same string in every language.

use crate::{__shipped_dictionary as shipped_dictionary, Dictionary};

shipped_dictionary!(
    /// Cryptocurrency full names and distinctive tickers (Bitcoin,
    /// BTC, Ethereum, DOGE, …) that rarely collide with ordinary
    /// text. Collision-prone terms live in
    /// [`cryptocurrencies_ambiguous`].
    fn cryptocurrencies
        from "../../../assets/dictionaries/world/finance/cryptocurrencies.toml"
        with csv "../../../assets/dictionaries/world/finance/cryptocurrencies.csv"
);
shipped_dictionary!(
    /// Cryptocurrency terms that collide with ordinary words (ADA,
    /// SOL, DOT, TON, …). Scored below threshold and gated on a crypto
    /// context keyword, so `Diabetes (ADA)` does not surface.
    fn cryptocurrencies_ambiguous
        from "../../../assets/dictionaries/world/finance/cryptocurrencies_ambiguous.toml"
        with csv "../../../assets/dictionaries/world/finance/cryptocurrencies_ambiguous.csv"
);
shipped_dictionary!(
    /// ISO 4217 fiat currency codes (USD, EUR, PLN, …). The
    /// language-agnostic counterpart to the English long-form
    /// currency names in `en::currencies`, so a currency code is still
    /// detected on a document whose asserted language is not English.
    /// Every three-letter code is ambiguous, so this dictionary is
    /// scored below threshold and gated on a currency context keyword.
    fn currency_codes
        from "../../../assets/dictionaries/world/finance/currency_codes.toml"
        with csv "../../../assets/dictionaries/world/finance/currency_codes.csv"
);

/// Every world-scoped built-in dictionary.
#[must_use]
pub fn all() -> Vec<Dictionary> {
    vec![
        cryptocurrencies(),
        cryptocurrencies_ambiguous(),
        currency_codes(),
    ]
}
