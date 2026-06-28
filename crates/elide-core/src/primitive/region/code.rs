//! ISO 3166-1 country code.

use std::fmt;
use std::str::FromStr;

use celes::Country;

/// [ISO 3166-1] country, identified by its code.
///
/// Wraps [`celes::Country`], a static table entry carrying the numeric,
/// alpha-2, and alpha-3 codes together with the country's name. Because
/// every value comes from that fixed table, a `CountryCode` is always a
/// real, recognised country; there is no way to hold an invalid one.
///
/// Used to scope region-sensitive recognizers (a phone-number or
/// national-id pattern, say) to the country whose format they target.
///
/// Serializes as its alpha-2 code (e.g. `"US"`).
///
/// [ISO 3166-1]: https://www.iso.org/iso-3166-country-codes.html
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(transparent))]
pub struct CountryCode(#[cfg_attr(feature = "schema", schemars(with = "String"))] Country);

impl CountryCode {
    /// Look up a country by its ISO 3166-1 alpha-2 code (e.g. `"US"`).
    pub fn from_alpha2(alpha2: impl AsRef<str>) -> Result<Self, &'static str> {
        Country::from_alpha2(alpha2).map(Self)
    }

    /// Look up a country by its ISO 3166-1 alpha-3 code (e.g. `"USA"`).
    pub fn from_alpha3(alpha3: impl AsRef<str>) -> Result<Self, &'static str> {
        Country::from_alpha3(alpha3).map(Self)
    }

    /// Country's ISO 3166-1 alpha-2 code (e.g. `"US"`): its canonical
    /// string form, matching [`Display`] and serde.
    ///
    /// [`Display`]: fmt::Display
    pub fn as_str(&self) -> &'static str {
        self.0.alpha2
    }

    /// Country's ISO 3166-1 alpha-3 code (e.g. `"USA"`).
    pub fn alpha3(&self) -> &'static str {
        self.0.alpha3
    }

    /// Underlying [`celes::Country`].
    pub fn country(&self) -> Country {
        self.0
    }
}

impl FromStr for CountryCode {
    type Err = &'static str;

    /// Accepts an alpha-2, alpha-3, numeric, or name form, per
    /// [`celes::Country`]'s [`FromStr`].
    fn from_str(code: &str) -> Result<Self, Self::Err> {
        Country::from_str(code).map(Self)
    }
}

impl fmt::Display for CountryCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0.alpha2)
    }
}
