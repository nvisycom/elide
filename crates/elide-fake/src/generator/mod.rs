//! Per-label fake-value generation, dispatched by [`Locale`].
//!
//! [`Context::generate`] returns `Some(string)` for every label the
//! catalogue covers, or `None` for labels the fake-data layer
//! doesn't support — the caller delegates to its fallback
//! anonymizer in that case.
//!
//! Two paths:
//!
//! - **Structured labels** (IBAN, payment cards, dates, IPs, …)
//!   pattern-preserve the original string: same length, same
//!   character-class layout, randomised digits and letters.
//!   See [`pattern::pattern_preserve`].
//! - **Free-form labels** (names, addresses, organisations, …)
//!   emit a fresh locale-aware fake whose length doesn't need to
//!   match. These go through per-domain submodules.

mod case_id;
mod contact;
mod device;
mod dispatch;
mod finance;
mod identity;
mod pattern;
mod temporal;

use fake::Fake;
use fake::faker::number::raw as number;
use fake::locales::EN;
use fake::rand::RngExt;

use crate::locale::Locale;

/// Per-call options threaded through to each label generator.
pub(crate) struct Context<'a> {
    locale: Locale,
    label: &'a str,
    original: &'a str,
}

impl<'a> Context<'a> {
    /// Build a generation request.
    pub(crate) fn new(locale: Locale, label: &'a str, original: &'a str) -> Self {
        Self {
            locale,
            label,
            original,
        }
    }

    /// Generate a fake replacement string for this context, using
    /// `rng` as the entropy source. Returns `None` when the entity
    /// label isn't covered.
    ///
    /// Two paths:
    /// - **Structured** labels reshape the original string in place
    ///   via [`pattern::pattern_preserve`]; they return `None` when
    ///   `original` is empty since there's no pattern to copy.
    /// - **Free-form** labels emit a fresh locale-aware fake whose
    ///   length doesn't need to match `original`.
    pub(crate) fn generate<R: RngExt + ?Sized>(self, rng: &mut R) -> Option<String> {
        let l = self.locale;
        let preserve = |rng: &mut R| {
            (!self.original.is_empty()).then(|| pattern::pattern_preserve(self.original, rng))
        };
        let value = match self.label {
            // identity (free-form)
            "person_name" => identity::person_name(l, rng),
            "organization_name" => identity::organization_name(l, rng),
            "occupation" => identity::occupation(l, rng),
            "username" => identity::username(l, rng),
            "gender" => identity::gender(l, rng),
            "language" => identity::language(rng),
            "nationality" => identity::nationality(l, rng),
            "citizenship" => identity::citizenship(l, rng),

            // contact
            "address" => contact::street_address(l, rng),
            "url" => contact::url(l, rng),
            "email_address" | "phone_number" | "postal_code" => return preserve(rng),

            // temporal
            "age" => temporal::age(rng),
            "date_of_birth" | "date_time" => return preserve(rng),

            // finance (free-form subset)
            "currency" => finance::currency_code(l, rng),
            "amount" => finance::amount(l, rng),
            "quantity" => finance::quantity(rng),

            // finance (structured)
            "iban" | "payment_card" | "card_security_code" | "card_expiry" | "bank_account"
            | "bank_routing" | "swift_code" | "crypto_address" => return preserve(rng),

            // device (free-form tokens)
            "password" => device::password(l, rng),
            "api_key" => device::api_key(rng),
            "auth_token" => device::auth_token(rng),
            "device_id" => device::device_id(rng),

            // device (structured)
            "ip_address" | "mac_address" | "coordinates" => return preserve(rng),

            // case ids (free-form)
            "internal_id" | "case_number" => case_id::internal_id(rng),

            // ids (structured)
            "government_id"
            | "tax_id"
            | "drivers_license"
            | "passport_number"
            | "national_insurance_number"
            | "vehicle_id"
            | "license_plate"
            | "medical_id"
            | "insurance_id"
            | "prescription_id" => return preserve(rng),

            _ => return None,
        };
        Some(value)
    }
}

/// Shared helper for labels that synthesise digit groups outside
/// the fake-rs locale tables (bank account, IDs).
pub(crate) fn digits<R: RngExt + ?Sized>(len: usize, rng: &mut R) -> String {
    let fmt = "#".repeat(len);
    number::NumberWithFormat(EN, fmt.as_str()).fake_with_rng(rng)
}

#[cfg(test)]
mod tests {
    use fake::rand::SeedableRng;
    use fake::rand::rngs::SmallRng;

    use super::*;

    fn rng() -> SmallRng {
        SmallRng::seed_from_u64(7)
    }

    fn ctx<'a>(locale: Locale, label: &'a str, original: &'a str) -> Context<'a> {
        Context::new(locale, label, original)
    }

    #[test]
    fn unsupported_labels_return_none() {
        let mut rng = rng();
        for label in ["fingerprint", "face", "religion", "diagnosis"] {
            assert!(
                ctx(Locale::En, label, "").generate(&mut rng).is_none(),
                "{label} should be None"
            );
        }
    }

    #[test]
    fn structured_label_with_empty_source_returns_none() {
        let mut rng = rng();
        // No pattern to copy → can't pattern-preserve.
        assert!(ctx(Locale::En, "iban", "").generate(&mut rng).is_none());
    }

    #[test]
    fn structured_labels_preserve_original_shape() {
        let cases: &[(&str, &str)] = &[
            ("iban", "GB82WEST12345698765432"),
            ("payment_card", "4111-1111-1111-1111"),
            ("phone_number", "+1-555-123-4567"),
            ("date_of_birth", "1985-03-12"),
            ("ip_address", "192.168.1.1"),
            ("postal_code", "SW1A 1AA"),
        ];
        for &(label, original) in cases {
            let mut rng = rng();
            let out = ctx(Locale::En, label, original).generate(&mut rng).unwrap();
            assert_eq!(out.len(), original.len(), "{label}: length mismatch");
            // Separator positions match.
            for (i, (a, b)) in out.chars().zip(original.chars()).enumerate() {
                if !a.is_ascii_alphanumeric() {
                    assert_eq!(a, b, "{label}: separator mismatch at {i} ({a:?} vs {b:?})");
                }
            }
        }
    }
}
