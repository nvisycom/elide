//! The fake-value catalogue: per-label generation, dispatched by [`Locale`].
//!
//! The internal engine both public wrappers drive through [`crate::synth`]:
//! [`Context::generate`] returns `Some(string)` for every label the catalogue
//! covers, or `None` for labels the fake-data layer doesn't support, so the
//! caller delegates to its fallback in that case.
//!
//! Two paths:
//!
//! - **Structured labels** (IBAN, payment cards, dates, IPs, …)
//!   pattern-preserve the original string: same length, same
//!   character-class layout, randomised digits and letters.
//!   See [`pattern::pattern_preserve`].
//! - **Free-form labels** (names, addresses, organisations, …)
//!   emit a fresh locale-aware fake whose length doesn't need to
//!   match. These go through per-category submodules (one per canonical
//!   label category).
//!
//! Some free-form generators read the [`Original`] to stay plausible in context:
//! an age keeps its life-stage band, a monetary amount its magnitude, a numeric
//! id its width.

mod contact;
mod credentials;
mod demographic;
mod dispatch;
mod financial;
mod geographic;
mod identity;
mod network;
mod organization;
mod original;
mod pattern;

use fake::Fake;
use fake::faker::number::raw as number;
use fake::locales::EN;
use fake::rand::RngExt;

pub(crate) use self::original::Original;
use crate::locale::Locale;

/// Per-call options threaded through to each label generator.
pub(crate) struct Context<'a> {
    locale: Locale,
    label: &'a str,
    original: Original<'a>,
}

impl<'a> Context<'a> {
    /// Build a generation request.
    pub(crate) fn new(locale: Locale, label: &'a str, original: &'a str) -> Self {
        Self {
            locale,
            label,
            original: Original::new(original),
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
            (!self.original.is_empty())
                .then(|| pattern::pattern_preserve(self.original.as_str(), rng))
        };
        // Arms are grouped by the label's canonical category (see
        // `elide_core::entity::label::builtins`); within a category, free-form
        // labels dispatch to that category's module and structured labels
        // pattern-preserve the original.
        let value = match self.label {
            // identity
            "person_name" => identity::person_name(l, rng),
            "government_id"
            | "tax_id"
            | "drivers_license"
            | "passport_number"
            | "national_insurance_number"
            | "vehicle_id"
            | "license_plate"
            | "certificate_number" => {
                return preserve(rng);
            }

            // contact
            "address" | "street_address" => contact::street_address(l, rng),
            "email_address" | "phone_number" | "fax_number" | "postal_code" => {
                return preserve(rng);
            }

            // geographic
            "city" => geographic::city(l, rng),
            "state" => geographic::state(l, rng),
            "country" => geographic::country(l, rng),
            "coordinates" | "precise_geolocation" | "geolocation_metadata" => return preserve(rng),

            // demographic
            "age" => demographic::age(&self.original, rng),
            "gender" => demographic::gender(l, rng),
            "language" => demographic::language(rng),
            "nationality" => demographic::nationality(l, rng),
            "citizenship" => demographic::citizenship(l, rng),

            // financial
            "currency" => financial::currency_code(l, rng),
            "monetary_amount" => financial::monetary_amount(&self.original, l, rng),
            "iban" | "payment_card" | "card_security_code" | "card_expiry" | "bank_account"
            | "bank_routing" | "swift_code" | "crypto_address" => return preserve(rng),

            // health (structured ids)
            "medical_id" | "insurance_id" | "prescription_id" => return preserve(rng),

            // credentials
            "password" => credentials::password(l, rng),
            "api_key" => credentials::api_key(rng),
            "auth_token" => credentials::auth_token(rng),

            // network
            "username" => network::username(l, rng),
            "url" => network::url(l, rng),
            "device_id" => network::device_id(rng),
            "ip_address" | "mac_address" => return preserve(rng),

            // organization
            "organization_name" => organization::organization_name(l, rng),
            "occupation" => organization::occupation(l, rng),
            "department_name" => organization::department_name(l, rng),
            "facility_name" => organization::facility_name(l, rng),
            "product" => organization::product(l, rng),
            "internal_id" | "case_number" => organization::internal_id(&self.original, rng),
            "company_id" => organization::company_id(&self.original, rng),

            // judicial (structured id)
            "court_case_number" => return preserve(rng),

            // contextual (structured)
            "date_of_birth" | "date_time" => return preserve(rng),

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
        // Labels with no believable fake, biometrics, free-text narratives, and
        // opaque artefacts, fall through so the caller masks them instead.
        let mut rng = rng();
        for label in [
            "fingerprint",
            "face",
            "diagnosis",
            "health_narrative",
            "barcode",
            // Special-category attributes: no believable locale-aware fake, and
            // masking is the more appropriate treatment.
            "religion",
            "ethnicity",
            "sexual_orientation",
        ] {
            assert!(
                ctx(Locale::En, label, "").generate(&mut rng).is_none(),
                "{label} should be None"
            );
        }
    }

    #[test]
    fn newly_covered_labels_generate_a_value() {
        // The categories added to the catalogue now produce a value rather than
        // falling through to the fallback.
        let mut rng = rng();
        for label in [
            "city",
            "state",
            "country",
            "street_address",
            "department_name",
            "facility_name",
            "product",
        ] {
            assert!(
                ctx(Locale::En, label, "").generate(&mut rng).is_some(),
                "{label} should generate a value"
            );
        }
        // Width-preserving numeric ids need a source to size against.
        for label in ["company_id", "fax_number"] {
            assert!(
                ctx(Locale::En, label, "12345").generate(&mut rng).is_some(),
                "{label} should generate a value"
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
