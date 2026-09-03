//! Organization category: company and department names, occupations, and the
//! operator-defined internal identifiers (internal id, case number).

use fake::faker::company::raw as company;
use fake::faker::job::raw as job;
use fake::rand::RngExt;

use super::dispatch::fan_locale;
use super::{Original, digits};
use crate::locale::Locale;

pub(super) fn organization_name<R: RngExt + ?Sized>(locale: Locale, rng: &mut R) -> String {
    fan_locale!(locale, rng, company::CompanyName)
}

pub(super) fn occupation<R: RngExt + ?Sized>(locale: Locale, rng: &mut R) -> String {
    fan_locale!(locale, rng, job::Position)
}

/// Department / business-unit name, an industry noun (`"Logistics"`,
/// `"Marketing"`) reads as a plausible department.
pub(super) fn department_name<R: RngExt + ?Sized>(locale: Locale, rng: &mut R) -> String {
    fan_locale!(locale, rng, company::Industry)
}

/// Facility or site name: a company name reads as a plausible facility label.
pub(super) fn facility_name<R: RngExt + ?Sized>(locale: Locale, rng: &mut R) -> String {
    fan_locale!(locale, rng, company::CompanyName)
}

/// Product name: a company buzzword-suffix phrase reads as a product.
pub(super) fn product<R: RngExt + ?Sized>(locale: Locale, rng: &mut R) -> String {
    fan_locale!(locale, rng, company::Buzzword)
}

/// Public company-registry identifier: an opaque numeric id whose width is
/// meaningful, so preserve the original's digit count (default 8).
pub(super) fn company_id<R: RngExt + ?Sized>(original: &Original<'_>, rng: &mut R) -> String {
    let width = original.digit_magnitude().unwrap_or(8);
    digits(width as usize, rng)
}

/// A numeric identifier for `InternalId` / `CaseNumber`, opaque ids without a
/// globally standardised format. Preserves the original's digit count, an id's
/// width is meaningful (a 6-digit case number is not a 12-digit one), so a
/// same-width replacement reads right; defaults to 10 digits when the original
/// carries none.
pub(super) fn internal_id<R: RngExt + ?Sized>(original: &Original<'_>, rng: &mut R) -> String {
    let width = original.digit_magnitude().unwrap_or(10);
    digits(width as usize, rng)
}
