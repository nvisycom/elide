//! Universal dictionaries — apply regardless of jurisdiction or
//! language.
//!
//! These are brand-name lists whose terms transcend locale.
//! Cryptocurrency names (`Bitcoin`, `Ethereum`) and tickers
//! (`BTC`, `ETH`) are the same string in every language.

use crate::{__shipped_dictionary as shipped_dictionary, Dictionary};

shipped_dictionary!(
    /// Cryptocurrency names and ticker symbols (BTC, Bitcoin, ETH,
    /// Ethereum, …).
    fn cryptocurrencies
        from "../../../assets/dictionaries/world/finance/cryptocurrencies.toml"
        with csv "../../../assets/dictionaries/world/finance/cryptocurrencies.csv"
);

/// Every world-scoped built-in dictionary.
#[must_use]
pub fn all() -> Vec<Dictionary> {
    vec![cryptocurrencies()]
}
