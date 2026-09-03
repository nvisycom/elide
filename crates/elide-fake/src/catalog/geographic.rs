//! Geographic category: place names, city, state, country. The precise
//! kinds (coordinates, precise geolocation, geolocation metadata) are structured
//! and pattern-preserve their original, so they don't go through this module.

use fake::faker::address::raw as address;
use fake::rand::RngExt;

use super::dispatch::fan_locale;
use crate::locale::Locale;

pub(super) fn city<R: RngExt + ?Sized>(locale: Locale, rng: &mut R) -> String {
    fan_locale!(locale, rng, address::CityName)
}

pub(super) fn state<R: RngExt + ?Sized>(locale: Locale, rng: &mut R) -> String {
    fan_locale!(locale, rng, address::StateName)
}

pub(super) fn country<R: RngExt + ?Sized>(locale: Locale, rng: &mut R) -> String {
    fan_locale!(locale, rng, address::CountryName)
}
