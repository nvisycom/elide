//! Identity category: a person's name. The identity documents
//! (government id, passport, driver's license, …) are structured and
//! pattern-preserve their original, so they don't go through this module.

use fake::faker::name::raw as name;
use fake::rand::RngExt;

use super::dispatch::fan_locale;
use crate::locale::Locale;

pub(super) fn person_name<R: RngExt + ?Sized>(locale: Locale, rng: &mut R) -> String {
    fan_locale!(locale, rng, name::Name)
}
