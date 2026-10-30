//! Built-in [`Pattern`] rules and [`Dictionary`]s shipped with this
//! crate.
//!
//! Each accessor parses an asset embedded via [`include_str!`] and
//! returns a fresh value. Dictionaries split metadata into a TOML
//! sidecar paired with a CSV/TXT term source; regex rules are
//! self-contained TOML. Call [`patterns::all`] / [`dictionaries::all`]
//! to load the full set, or pick individual accessors.
//!
//! [`Pattern`]: crate::Pattern
//! [`Dictionary`]: crate::Dictionary

pub mod dictionaries;
pub mod patterns;
