//! Built-in [`Label`] constants.
//!
//! Each constant carries a category tag (`personal_identity`,
//! `financial`, …) plus cross-cutting tags where applicable (`pii`,
//! `phi`, `pci`). Selectors can match by label id *or* by tag without
//! the workspace modelling categories as a separate enum.
//!
//! Every label's id is its constant's identifier, lowercased
//! (`PHONE_NUMBER` → `"phone_number"`); the display `name` is the
//! GLiNER-style natural-language phrase.
//!
//! The `BUILT_INS` slice indexes every constant for the
//! [`LabelCatalog::with_builtins`] constructor; the constants themselves
//! are public (e.g. `builtins::PERSON_NAME`).
//!
//! [`LabelCatalog::with_builtins`]: super::LabelCatalog::with_builtins

use std::sync::LazyLock;

use super::Label;

/// A built-in label with a display `name` and tags — no description. The
/// common case; use [`rich_label`] to add a description.
///
/// The id is the constant's identifier, lowercased
/// (`PHONE_NUMBER` → `"phone_number"`) — the same lowercase string the
/// shipped pattern-rule `.toml` assets reference in their `label` field, so
/// a rule's emitted [`LabelRef`] resolves against this label.
macro_rules! label {
    ($vis:vis $ident:ident, $name:literal, [ $($tag:literal),* $(,)? ]) => {
        #[doc = $name]
        $vis static $ident: LazyLock<Label> = LazyLock::new(|| {
            Label::from_static(
                stringify!($ident).to_ascii_lowercase(),
                $name,
                None,
                &[$($tag),*],
            )
        });
    };
}

/// A built-in label that also carries a fuller `description`, for
/// description-capable backends (GLiNER-2.0, LLM). Id derived as in
/// [`label`].
macro_rules! rich_label {
    ($vis:vis $ident:ident, $name:literal, $desc:literal, [ $($tag:literal),* $(,)? ]) => {
        #[doc = $desc]
        $vis static $ident: LazyLock<Label> = LazyLock::new(|| {
            Label::from_static(
                stringify!($ident).to_ascii_lowercase(),
                $name,
                Some($desc),
                &[$($tag),*],
            )
        });
    };
}

label!(pub PERSON_NAME, "person name", ["personal_identity", "pii"]);
label!(pub DATE_OF_BIRTH, "date of birth", ["personal_identity", "pii"]);
rich_label!(pub GOVERNMENT_ID, "government-issued identification number", "A government-issued identification number such as a US SSN, Canadian SIN, Indian Aadhaar, or other national identity number.", ["personal_identity", "pii"]);
rich_label!(pub TAX_ID, "tax identification number", "A taxpayer identification number such as a US ITIN or EIN, a VAT number, or another jurisdiction's tax id.", ["personal_identity", "pii"]);
label!(pub DRIVERS_LICENSE, "driver's license number", ["personal_identity", "pii"]);
label!(pub PASSPORT_NUMBER, "passport number", ["personal_identity", "pii"]);
label!(pub NATIONAL_INSURANCE_NUMBER, "national insurance or social-security equivalent", ["personal_identity", "pii"]);
label!(pub VEHICLE_ID, "vehicle identification number", ["personal_identity"]);
label!(pub LICENSE_PLATE, "license plate number", ["personal_identity"]);
label!(pub EMAIL_ADDRESS, "email address", ["contact_info", "pii"]);
label!(pub PHONE_NUMBER, "phone number", ["contact_info", "pii"]);
label!(pub ADDRESS, "physical or mailing address", ["contact_info", "pii"]);
label!(pub POSTAL_CODE, "postal or ZIP code", ["contact_info"]);
label!(pub URL, "URL or hyperlink", ["contact_info"]);
label!(pub AGE, "age value", ["demographic", "pii"]);
label!(pub GENDER, "gender identity", ["demographic", "pii"]);
label!(pub ETHNICITY, "racial or ethnic background", ["demographic", "pii"]);
label!(pub RELIGION, "religious affiliation", ["demographic", "pii"]);
label!(pub NATIONALITY, "nationality", ["demographic", "pii"]);
label!(pub CITIZENSHIP, "citizenship status", ["demographic", "pii"]);
label!(pub LANGUAGE, "language or dialect spoken", ["demographic"]);
label!(pub PAYMENT_CARD, "payment card number", ["financial", "pci", "pii"]);
label!(pub CARD_SECURITY_CODE, "payment card security code", ["financial", "pci"]);
label!(pub CARD_EXPIRY, "payment card expiration date", ["financial", "pci"]);
label!(pub BANK_ACCOUNT, "bank account number", ["financial", "pii"]);
label!(pub BANK_ROUTING, "bank routing or transit number", ["financial"]);
label!(pub IBAN, "international bank account number", ["financial", "pii"]);
label!(pub SWIFT_CODE, "SWIFT/BIC code", ["financial"]);
label!(pub CRYPTO_ADDRESS, "cryptocurrency wallet address", ["financial", "pii"]);
label!(pub CURRENCY, "currency code or symbol", ["financial"]);
label!(pub AMOUNT, "monetary amount", ["financial"]);
label!(pub MEDICAL_ID, "medical record number", ["health", "phi", "pii"]);
label!(pub INSURANCE_ID, "health insurance identifier", ["health", "phi", "pii"]);
label!(pub PRESCRIPTION_ID, "prescription identifier or medication regimen", ["health", "phi"]);
label!(pub DIAGNOSIS, "medical diagnosis or condition", ["health", "phi"]);
label!(pub MEDICATION, "medication name", ["health", "phi"]);
label!(pub FINGERPRINT, "fingerprint biometric data", ["biometric", "pii"]);
label!(pub VOICEPRINT, "voiceprint biometric data", ["biometric", "pii"]);
label!(pub RETINA_SCAN, "retina scan biometric data", ["biometric", "pii"]);
label!(pub FACIAL_GEOMETRY, "facial geometry biometric data", ["biometric", "pii"]);
label!(pub PASSWORD, "password", ["credentials", "secret"]);
label!(pub API_KEY, "API key", ["credentials", "secret"]);
label!(pub AUTH_TOKEN, "authentication token", ["credentials", "secret"]);
label!(pub PRIVATE_KEY, "private cryptographic key", ["credentials", "secret"]);
label!(pub IP_ADDRESS, "IP address", ["network_identifier", "pii"]);
label!(pub MAC_ADDRESS, "MAC address", ["network_identifier", "pii"]);
label!(pub DEVICE_ID, "device identifier", ["network_identifier", "pii"]);
label!(pub USERNAME, "username or handle", ["network_identifier", "pii"]);
label!(pub COORDINATES, "GPS coordinates", ["location", "pii"]);
label!(pub GEOLOCATION_METADATA, "geolocation metadata", ["location", "pii"]);
label!(pub FACE, "human face detected in an image or video frame", ["visual", "pii"]);
label!(pub HANDWRITING, "handwritten text", ["visual"]);
label!(pub SIGNATURE, "handwritten signature", ["visual", "pii"]);
label!(pub LOGO, "brand or organisation logo", ["visual"]);
label!(pub BARCODE, "barcode or QR code", ["visual"]);
label!(pub ORGANIZATION_NAME, "organization or company name", ["organization"]);
label!(pub COMPANY_ID, "public company-registry identifier", ["organization"]);
label!(pub DEPARTMENT_NAME, "department or business-unit name", ["organization"]);
label!(pub FACILITY_NAME, "physical facility or location name", ["organization"]);
label!(pub CASE_NUMBER, "case, matter, or docket number", ["organization"]);
label!(pub INTERNAL_ID, "operator-defined internal identifier", ["organization"]);
label!(pub DATE_TIME, "date or time value", ["temporal"]);
label!(pub EVENT, "named event reference", ["temporal"]);
label!(pub OCCUPATION, "occupation or job title", ["organization"]);
label!(pub PRODUCT, "product name", ["organization"]);
label!(pub QUANTITY, "numerical quantity", ["quantity"]);
rich_label!(pub UNRESOLVED, "unresolved entity", "A sensitive entity whose specific type has not been resolved; a catch-all for detections that do not fit a more precise label.", ["unresolved"]);

/// Every built-in label constant, indexed for catalog construction.
pub(super) static BUILT_INS: &[&LazyLock<Label>] = &[
    &PERSON_NAME,
    &DATE_OF_BIRTH,
    &GOVERNMENT_ID,
    &TAX_ID,
    &DRIVERS_LICENSE,
    &PASSPORT_NUMBER,
    &NATIONAL_INSURANCE_NUMBER,
    &VEHICLE_ID,
    &LICENSE_PLATE,
    &EMAIL_ADDRESS,
    &PHONE_NUMBER,
    &ADDRESS,
    &POSTAL_CODE,
    &URL,
    &AGE,
    &GENDER,
    &ETHNICITY,
    &RELIGION,
    &NATIONALITY,
    &CITIZENSHIP,
    &LANGUAGE,
    &PAYMENT_CARD,
    &CARD_SECURITY_CODE,
    &CARD_EXPIRY,
    &BANK_ACCOUNT,
    &BANK_ROUTING,
    &IBAN,
    &SWIFT_CODE,
    &CRYPTO_ADDRESS,
    &CURRENCY,
    &AMOUNT,
    &MEDICAL_ID,
    &INSURANCE_ID,
    &PRESCRIPTION_ID,
    &DIAGNOSIS,
    &MEDICATION,
    &FINGERPRINT,
    &VOICEPRINT,
    &RETINA_SCAN,
    &FACIAL_GEOMETRY,
    &PASSWORD,
    &API_KEY,
    &AUTH_TOKEN,
    &PRIVATE_KEY,
    &IP_ADDRESS,
    &MAC_ADDRESS,
    &DEVICE_ID,
    &USERNAME,
    &COORDINATES,
    &GEOLOCATION_METADATA,
    &FACE,
    &HANDWRITING,
    &SIGNATURE,
    &LOGO,
    &BARCODE,
    &ORGANIZATION_NAME,
    &COMPANY_ID,
    &DEPARTMENT_NAME,
    &FACILITY_NAME,
    &CASE_NUMBER,
    &INTERNAL_ID,
    &DATE_TIME,
    &EVENT,
    &OCCUPATION,
    &PRODUCT,
    &QUANTITY,
    &UNRESOLVED,
];

#[cfg(test)]
mod tests {
    use crate::primitive::LanguageTag;

    use super::*;

    #[test]
    fn well_known_built_ins_have_expected_ids_names_and_tags() {
        let en = LanguageTag::english();

        // The id is the constant's identifier, lowercased; the name is the
        // GLiNER-style natural-language phrase.
        assert_eq!(PAYMENT_CARD.id(), "payment_card");
        assert_eq!(PAYMENT_CARD.name(&en), "payment card number");
        assert!(PAYMENT_CARD.has_tag("financial"));
        assert!(PAYMENT_CARD.has_tag("pci"));
        assert!(PAYMENT_CARD.has_tag("pii"));

        assert_eq!(PERSON_NAME.id(), "person_name");
        assert_eq!(PERSON_NAME.name(&en), "person name");
        assert!(PERSON_NAME.has_tag("personal_identity"));

        // A `rich_label!` carries a description; a plain `label!` does not.
        assert!(GOVERNMENT_ID.description(&en).is_some());
        assert!(PERSON_NAME.description(&en).is_none());
    }
}
