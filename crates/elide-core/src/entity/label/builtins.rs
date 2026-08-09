//! Built-in [`Label`] constants.
//!
//! Each constant carries a category tag (`personal_identity`,
//! `financial`, …) plus cross-cutting tags where applicable (`pii`,
//! `phi`, `pci`, `sad` for PCI sensitive authentication data). Selectors
//! can match by label id *or* by tag without the workspace modelling
//! categories as a separate enum.
//!
//! Tags name *what the data is*, not which law governs it: mapping a
//! category to a regulatory regime (GDPR Article 9, HIPAA Safe Harbor, …)
//! is a policy-layer concern, so a compliance profile selects the relevant
//! category tags itself rather than the catalog carrying regulation-citation
//! tags.
//!
//! Every label's id is its constant's identifier, lowercased
//! (`PHONE_NUMBER` → `"phone_number"`); the display `name` is the
//! GLiNER-style natural-language phrase.
//!
//! The constants and their `BUILT_INS` index are generated together from one
//! `labels!` table, grouped by category, so the index can never drift from the
//! definitions. [`LabelCatalog::with_builtins`] walks that index; the constants
//! themselves are public (e.g. [`PERSON_NAME`]).
//!
//! [`LabelCatalog::with_builtins`]: super::LabelCatalog::with_builtins

use std::sync::LazyLock;

use super::Label;

/// Define the built-in labels as one grouped table, emitting a
/// `LazyLock<Label>` constant per entry *and* the `BUILT_INS` slice indexing
/// them all, so the two can never fall out of sync.
///
/// Each entry is `NAME = "display name"[ : "description"], [tags…];`. The
/// optional `: "description"` makes it a rich label (a description for
/// description-capable backends); without it the name stands alone. The id is
/// the constant's identifier lowercased (`PHONE_NUMBER` becomes
/// `"phone_number"`), the same lowercase string the shipped pattern-rule
/// `.toml` assets reference in their `label` field, so a rule's emitted
/// [`LabelRef`] resolves against this label.
///
/// `// group` comments in the table are ordinary comments; they organize the
/// entries visually but don't affect what is generated.
///
/// [`LabelRef`]: super::LabelRef
macro_rules! labels {
    (
        $(
            $ident:ident = $name:literal $(: $desc:literal)? , [ $($tag:literal),* $(,)? ] ;
        )*
    ) => {
        $(
            #[doc = $name]
            pub static $ident: LazyLock<Label> = LazyLock::new(|| {
                #[allow(unused_mut, unused_assignments)]
                let mut description: Option<&'static str> = None;
                $( description = Some($desc); )?
                Label::from_static(
                    stringify!($ident).to_ascii_lowercase(),
                    $name,
                    description,
                    &[$($tag),*],
                )
            });
        )*

        /// Every built-in label constant, indexed for catalog construction.
        ///
        /// Generated from the same [`labels!`] table as the constants, so it
        /// always lists exactly them, with no manual upkeep.
        pub(super) static BUILT_INS: &[&LazyLock<Label>] = &[
            $( &$ident, )*
        ];
    };
}

labels! {
    // Personal identity
    PERSON_NAME = "person name", ["personal_identity", "pii"];
    DATE_OF_BIRTH = "date of birth", ["personal_identity", "pii"];
    GOVERNMENT_ID = "government-issued identification number"
        : "Government-issued identification number such as a US SSN, Canadian SIN, Indian Aadhaar, or other national identity number.",
        ["personal_identity", "pii"];
    TAX_ID = "tax identification number"
        : "Taxpayer identification number such as a US ITIN or EIN, a VAT number, or another jurisdiction's tax id.",
        ["personal_identity", "pii"];
    DRIVERS_LICENSE = "driver's license number", ["personal_identity", "pii"];
    CERTIFICATE_NUMBER = "certificate or license number"
        : "Regulated identifier issued by a professional, licensing, or certifying body, such as a DEA registration number, medical license, bar number, or notary commission.",
        ["personal_identity", "pii"];
    PASSPORT_NUMBER = "passport number", ["personal_identity", "pii"];
    NATIONAL_INSURANCE_NUMBER = "national insurance or social-security equivalent", ["personal_identity", "pii"];
    VEHICLE_ID = "vehicle identification number", ["personal_identity", "pii"];
    LICENSE_PLATE = "license plate number", ["personal_identity", "pii"];

    // Contact information
    EMAIL_ADDRESS = "email address", ["contact_info", "pii"];
    PHONE_NUMBER = "phone number", ["contact_info", "pii"];
    FAX_NUMBER = "fax number", ["contact_info", "pii"];
    ADDRESS = "physical or mailing address", ["contact_info", "pii"];
    STREET_ADDRESS = "street address line"
        : "The street line of a physical address (number, street name, unit), excluding town/city, state, and postal code. The finest-grained slice of an address split by geographic granularity.",
        ["contact_info", "pii"];
    CITY = "town or city name", ["contact_info", "pii"];
    STATE = "state or province", ["contact_info"];
    COUNTRY = "country name", ["contact_info"];
    POSTAL_CODE = "postal or ZIP code", ["contact_info"];
    COMMUNICATIONS_CONTENT = "communication message content"
        : "Free-form body content of a personal communication (mail, email, SMS, or chat message), as distinct from the header identifiers (address, phone) that route it.",
        ["contact_info", "pii"];

    // Demographic
    AGE = "age value", ["demographic", "pii"];
    GENDER = "gender identity", ["demographic", "pii"];
    ETHNICITY = "racial or ethnic background", ["demographic", "pii"];
    RELIGION = "religious affiliation", ["demographic", "pii"];
    NATIONALITY = "nationality", ["demographic", "pii"];
    CITIZENSHIP = "citizenship status", ["demographic", "pii"];
    LANGUAGE = "language or dialect spoken", ["demographic"];
    POLITICAL_OPINION = "political opinion or affiliation", ["demographic", "pii"];
    TRADE_UNION_MEMBERSHIP = "trade-union membership", ["demographic", "pii"];
    SEXUAL_ORIENTATION = "sexual orientation", ["demographic", "pii"];
    SEX_LIFE = "sex-life information"
        : "Narrative content describing a person's sex life, as distinct from sexual-orientation identity.",
        ["demographic", "pii"];
    EDUCATION_RECORD = "education record entry"
        : "Grade, transcript, disciplinary, or enrollment record for a person.",
        ["demographic", "pii"];
    INFERENCE = "profile inference"
        : "Model-derived characteristic inferred about a person to build a profile (preferences, psychological trends, predispositions, aptitudes), rather than a surface-level entity.",
        ["demographic", "pii"];

    // Financial
    PAYMENT_CARD = "payment card number", ["financial", "pci", "pii"];
    CARD_SECURITY_CODE = "payment card security code", ["financial", "pci", "sad"];
    CARD_TRACK_DATA = "payment card track data"
        : "Magnetic-stripe or chip track data (Track 1 / Track 2 contents) from a payment card. Sensitive authentication data that must not be retained after authorization.",
        ["financial", "pci", "sad"];
    PIN_BLOCK = "payment card PIN or PIN block"
        : "Payment card PIN or encrypted PIN block. Sensitive authentication data that must not be retained after authorization.",
        ["financial", "pci", "sad"];
    CARD_EXPIRY = "payment card expiration date", ["financial", "pci"];
    BANK_ACCOUNT = "bank account number", ["financial", "pii"];
    BANK_ROUTING = "bank routing or transit number", ["financial"];
    IBAN = "international bank account number", ["financial", "pii"];
    SWIFT_CODE = "SWIFT/BIC code", ["financial"];
    CRYPTO_ADDRESS = "cryptocurrency wallet address", ["financial", "pii"];
    CURRENCY = "currency code or symbol", ["financial"];
    AMOUNT = "monetary amount", ["financial"];

    // Health
    MEDICAL_ID = "medical record number", ["health", "phi", "pii"];
    INSURANCE_ID = "health insurance identifier", ["health", "phi", "pii"];
    PRESCRIPTION_ID = "prescription identifier or medication regimen", ["health", "phi"];
    DIAGNOSIS = "medical diagnosis or condition", ["health", "phi"];
    MEDICATION = "medication name", ["health", "phi"];
    HEALTH_NARRATIVE = "health narrative text"
        : "Free-form clinical or therapy text that reveals a person's physical or mental health status without being a specific identifier, diagnosis, or medication, such as vital readings, appointment context, care plans, or therapist references.",
        ["health", "phi"];

    // Biometric
    FINGERPRINT = "fingerprint biometric data", ["biometric", "pii"];
    VOICEPRINT = "voiceprint biometric data", ["biometric", "pii"];
    RETINA_SCAN = "retina scan biometric data", ["biometric", "pii"];
    FACIAL_GEOMETRY = "facial geometry biometric data", ["biometric", "pii"];
    GENETIC_DATA = "genetic data", ["biometric", "pii"];

    // Credentials
    PASSWORD = "password", ["credentials", "secret"];
    SECURITY_QUESTION_ANSWER = "account security question or answer"
        : "Knowledge-based challenge question or its answer used to recover or verify an account.",
        ["credentials", "secret"];
    API_KEY = "API key", ["credentials", "secret"];
    AUTH_TOKEN = "authentication token", ["credentials", "secret"];
    PRIVATE_KEY = "private cryptographic key", ["credentials", "secret"];

    // Network identifiers
    URL = "URL or hyperlink", ["network_identifier"];
    IP_ADDRESS = "IP address", ["network_identifier", "pii"];
    MAC_ADDRESS = "MAC address", ["network_identifier", "pii"];
    DEVICE_ID = "device identifier", ["network_identifier", "pii"];
    USERNAME = "username or handle", ["network_identifier", "pii"];

    // Location
    COORDINATES = "GPS coordinates", ["location", "pii"];
    PRECISE_GEOLOCATION = "precise geolocation"
        : "Geolocation pinpointing a person to a small radius (roughly a city block or finer), as distinct from an approximate or region-level location.",
        ["location", "pii"];
    GEOLOCATION_METADATA = "geolocation metadata", ["location", "pii"];

    // Visual
    FACE = "human face detected in an image or video frame", ["visual", "pii"];
    HANDWRITING = "handwritten text", ["visual"];
    SIGNATURE = "handwritten signature", ["visual", "pii"];
    LOGO = "brand or organisation logo", ["visual"];
    BARCODE = "barcode or QR code", ["visual"];

    // Organization
    ORGANIZATION_NAME = "organization or company name", ["organization"];
    COMPANY_ID = "public company-registry identifier", ["organization"];
    DEPARTMENT_NAME = "department or business-unit name", ["organization"];
    FACILITY_NAME = "physical facility or location name", ["organization"];
    CASE_NUMBER = "case, matter, or docket number", ["organization"];
    INTERNAL_ID = "operator-defined internal identifier", ["organization"];
    OCCUPATION = "occupation or job title", ["organization"];
    PRODUCT = "product name", ["organization"];

    // Temporal
    DATE_TIME = "date or time value", ["temporal"];
    INDIVIDUAL_DATE = "individual-associated date"
        : "Date directly relating to a natural person (birth, admission, discharge, death, or service date), as distinct from a bare calendar date such as an invoice or meeting date.",
        ["temporal", "pii"];
    EVENT = "named event reference", ["temporal"];

    // Miscellaneous
    UNRESOLVED = "unresolved entity"
        : "Sensitive entity whose specific type has not been resolved; a catch-all for detections that do not fit a more precise label.",
        ["unresolved"];
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitive::LanguageTag;

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

        // A rich entry (`: "…"`) carries a description; a plain one does not.
        assert!(GOVERNMENT_ID.description(&en).is_some());
        assert!(PERSON_NAME.description(&en).is_none());
    }

    #[test]
    fn built_ins_index_matches_the_constants() {
        // The `labels!` table generates BUILT_INS from the same entries as the
        // constants, so ids are unique and every constant is indexed.
        let mut ids: Vec<&str> = BUILT_INS.iter().map(|l| l.id()).collect();
        let count = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), count, "built-in label ids must be unique");
    }

    #[test]
    fn special_category_labels_ship_as_data_categories() {
        let en = LanguageTag::english();

        // The categories GDPR Art. 9 covers that the catalog previously
        // lacked. They carry ordinary *data-category* tags (demographic,
        // biometric), no regulation-citation tag, since mapping a category
        // to a legal regime is a policy-layer concern.
        assert_eq!(POLITICAL_OPINION.id(), "political_opinion");
        assert_eq!(
            POLITICAL_OPINION.name(&en),
            "political opinion or affiliation"
        );
        assert!(POLITICAL_OPINION.has_tag("demographic"));

        assert_eq!(TRADE_UNION_MEMBERSHIP.id(), "trade_union_membership");
        assert!(TRADE_UNION_MEMBERSHIP.has_tag("demographic"));

        assert_eq!(SEXUAL_ORIENTATION.id(), "sexual_orientation");
        assert!(SEXUAL_ORIENTATION.has_tag("demographic"));

        assert_eq!(GENETIC_DATA.id(), "genetic_data");
        assert!(GENETIC_DATA.has_tag("biometric"));

        // No regulation-citation tag leaked onto the taxonomy.
        for label in [
            &*POLITICAL_OPINION,
            &TRADE_UNION_MEMBERSHIP,
            &SEXUAL_ORIENTATION,
            &GENETIC_DATA,
        ] {
            assert!(
                !label.has_tag("article_9"),
                "regulatory tags belong to the policy layer"
            );
        }
    }

    #[test]
    fn sad_tag_isolates_sensitive_authentication_data() {
        use super::super::LabelCatalog;

        // PCI SAD (must never be stored post-auth) is a subset of PCI scope;
        // the `sad` tag isolates it from the rest.
        let sad = LabelCatalog::with_builtins().filter_tag("sad");
        assert!(sad.contains(&CARD_SECURITY_CODE.to_ref()));
        assert!(sad.contains(&CARD_TRACK_DATA.to_ref()));
        assert!(sad.contains(&PIN_BLOCK.to_ref()));
        // `sad` is additive on top of the existing PCI/financial tags.
        assert!(CARD_SECURITY_CODE.has_tag("pci") && CARD_SECURITY_CODE.has_tag("sad"));
        // The PAN is PCI but not SAD (it may be stored, masked).
        assert!(PAYMENT_CARD.has_tag("pci") && !PAYMENT_CARD.has_tag("sad"));
    }

    #[test]
    fn hipaa_fax_and_certificate_labels_ship() {
        let en = LanguageTag::english();

        assert_eq!(FAX_NUMBER.id(), "fax_number");
        assert_eq!(FAX_NUMBER.name(&en), "fax number");
        assert!(FAX_NUMBER.has_tag("contact_info") && FAX_NUMBER.has_tag("pii"));

        assert_eq!(CERTIFICATE_NUMBER.id(), "certificate_number");
        // `certificate_number` is ambiguous, so it ships with a description.
        assert!(CERTIFICATE_NUMBER.description(&en).is_some());
    }

    #[test]
    fn geographic_granularity_labels_split_the_address() {
        let en = LanguageTag::english();

        // The address blob splits by granularity so a survivor set can keep
        // coarser components (state) while dropping finer ones (street).
        for label in [&*STREET_ADDRESS, &CITY, &STATE, &COUNTRY] {
            assert!(label.has_tag("contact_info"));
        }
        assert_eq!(STREET_ADDRESS.id(), "street_address");
        assert!(STREET_ADDRESS.description(&en).is_some());
        // State and country survive coarser cuts, so they aren't tagged `pii`.
        assert!(!STATE.has_tag("pii"));
        assert!(!COUNTRY.has_tag("pii"));
    }
}
