//! Built-in [`Label`] constants.
//!
//! Each constant carries a [`Category`](super::Category) (`identity`,
//! `financial`, `health`, …), the coarse group it belongs to for organizing
//! detected entities, plus
//! cross-cutting tags where applicable (`pii`, `phi`, `pci`, `sad` for PCI
//! sensitive authentication data). Selectors can match by label id, by tag, or
//! group by category.
//!
//! Category and tags both name *what the data is*, not which law governs it:
//! mapping a category to a regulatory regime (GDPR Article 9, HIPAA Safe
//! Harbor, …) is a policy-layer concern, so a compliance profile selects the
//! relevant categories or tags itself rather than the catalog carrying
//! regulation-citation tags.
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
/// Each entry is `NAME @ category = "display name"[ : "description"], [tags…];`.
/// The `@ category` is the label's coarse group; the optional `: "description"`
/// makes it a rich label (a description for description-capable backends);
/// `[tags…]` are cross-cutting sensitivity markers (`pii`, `phi`, …), the
/// category excluded. The id is the constant's identifier lowercased
/// (`PHONE_NUMBER` becomes `"phone_number"`), the same lowercase string the
/// shipped pattern-rule `.toml` assets reference in their `label` field, so a
/// rule's emitted [`LabelRef`] resolves against this label.
///
/// `// group` comments in the table are ordinary comments; they organize the
/// entries visually but don't affect what is generated.
///
/// [`LabelRef`]: super::LabelRef
macro_rules! labels {
    (
        $(
            $ident:ident @ $category:literal = $name:literal $(: $desc:literal)? , [ $($tag:literal),* $(,)? ] ;
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
                    $category,
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
    // Identity: names and identity documents.
    PERSON_NAME @ "identity" = "person name", ["pii"];
    DATE_OF_BIRTH @ "identity" = "date of birth", ["pii"];
    GOVERNMENT_ID @ "identity" = "government-issued identification number"
        : "Government-issued identification number such as a US SSN, Canadian SIN, Indian Aadhaar, or other national identity number.",
        ["pii"];
    TAX_ID @ "identity" = "tax identification number"
        : "Taxpayer identification number such as a US ITIN or EIN, a VAT number, or another jurisdiction's tax id.",
        ["pii"];
    DRIVERS_LICENSE @ "identity" = "driver's license number", ["pii"];
    CERTIFICATE_NUMBER @ "identity" = "certificate or license number"
        : "Regulated identifier issued by a professional, licensing, or certifying body, such as a DEA registration number, medical license, bar number, or notary commission.",
        ["pii"];
    PASSPORT_NUMBER @ "identity" = "passport number", ["pii"];
    NATIONAL_INSURANCE_NUMBER @ "identity" = "national insurance or social-security equivalent", ["pii"];
    VEHICLE_ID @ "identity" = "vehicle identification number", ["pii"];
    LICENSE_PLATE @ "identity" = "license plate number", ["pii"];
    SIGNATURE @ "identity" = "handwritten signature", ["pii"];

    // Contact: ways to reach a person.
    EMAIL_ADDRESS @ "contact" = "email address", ["pii"];
    PHONE_NUMBER @ "contact" = "phone number", ["pii"];
    FAX_NUMBER @ "contact" = "fax number", ["pii"];
    ADDRESS @ "contact" = "physical or mailing address", ["pii"];
    STREET_ADDRESS @ "contact" = "street address line"
        : "The street line of a physical address (number, street name, unit), excluding town/city, state, and postal code. The finest-grained slice of an address split by geographic granularity.",
        ["pii"];
    POSTAL_CODE @ "contact" = "postal or ZIP code", [];
    COMMUNICATIONS_CONTENT @ "contact" = "communication message content"
        : "Free-form body content of a personal communication (mail, email, SMS, or chat message), as distinct from the header identifiers (address, phone) that route it.",
        ["pii"];

    // Geographic: places and geolocation.
    CITY @ "geographic" = "town or city name", ["pii"];
    STATE @ "geographic" = "state or province", [];
    COUNTRY @ "geographic" = "country name", [];
    COORDINATES @ "geographic" = "GPS coordinates", ["pii"];
    PRECISE_GEOLOCATION @ "geographic" = "precise geolocation"
        : "Geolocation pinpointing a person to a small radius (roughly a city block or finer), as distinct from an approximate or region-level location.",
        ["pii"];
    GEOLOCATION_METADATA @ "geographic" = "geolocation metadata", ["pii"];

    // Demographic: personal attributes.
    AGE @ "demographic" = "age value", ["pii"];
    GENDER @ "demographic" = "gender identity", ["pii"];
    NATIONALITY @ "demographic" = "nationality", ["pii"];
    CITIZENSHIP @ "demographic" = "citizenship status", ["pii"];
    LANGUAGE @ "demographic" = "language or dialect spoken", [];

    // Protected characteristic: special-category attributes.
    ETHNICITY @ "protected_characteristic" = "racial or ethnic background", ["pii"];
    RELIGION @ "protected_characteristic" = "religious affiliation", ["pii"];
    POLITICAL_OPINION @ "protected_characteristic" = "political opinion or affiliation", ["pii"];
    TRADE_UNION_MEMBERSHIP @ "protected_characteristic" = "trade-union membership", ["pii"];
    SEXUAL_ORIENTATION @ "protected_characteristic" = "sexual orientation", ["pii"];
    SEX_LIFE @ "protected_characteristic" = "sex-life information"
        : "Narrative content describing a person's sex life, as distinct from sexual-orientation identity.",
        ["pii"];

    // Financial: money and payment instruments.
    PAYMENT_CARD @ "financial" = "payment card number", ["pci", "pii"];
    CARD_SECURITY_CODE @ "financial" = "payment card security code", ["pci", "sad"];
    CARD_TRACK_DATA @ "financial" = "payment card track data"
        : "Magnetic-stripe or chip track data (Track 1 / Track 2 contents) from a payment card. Sensitive authentication data that must not be retained after authorization.",
        ["pci", "sad"];
    PIN_BLOCK @ "financial" = "payment card PIN or PIN block"
        : "Payment card PIN or encrypted PIN block. Sensitive authentication data that must not be retained after authorization.",
        ["pci", "sad"];
    CARD_EXPIRY @ "financial" = "payment card expiration date", ["pci"];
    BANK_ACCOUNT @ "financial" = "bank account number", ["pii"];
    BANK_ROUTING @ "financial" = "bank routing or transit number", [];
    IBAN @ "financial" = "international bank account number", ["pii"];
    SWIFT_CODE @ "financial" = "SWIFT/BIC code", [];
    CRYPTO_ADDRESS @ "financial" = "cryptocurrency wallet address", ["pii"];
    CURRENCY @ "financial" = "currency code or symbol", [];
    AMOUNT @ "financial" = "monetary amount", [];

    // Health: medical and health-status information.
    MEDICAL_ID @ "health" = "medical record number", ["phi", "pii"];
    INSURANCE_ID @ "health" = "health insurance identifier", ["phi", "pii"];
    PRESCRIPTION_ID @ "health" = "prescription identifier or medication regimen", ["phi"];
    DIAGNOSIS @ "health" = "medical diagnosis or condition", ["phi"];
    MEDICATION @ "health" = "medication name", ["phi"];
    HEALTH_NARRATIVE @ "health" = "health narrative text"
        : "Free-form clinical or therapy text that reveals a person's physical or mental health status without being a specific identifier, diagnosis, or medication, such as vital readings, appointment context, care plans, or therapist references.",
        ["phi"];

    // Biometric: biometric identifiers.
    FINGERPRINT @ "biometric" = "fingerprint biometric data", ["pii"];
    VOICEPRINT @ "biometric" = "voiceprint biometric data", ["pii"];
    RETINA_SCAN @ "biometric" = "retina scan biometric data", ["pii"];
    FACIAL_GEOMETRY @ "biometric" = "facial geometry biometric data", ["pii"];
    GENETIC_DATA @ "biometric" = "genetic data", ["pii"];
    FACE @ "biometric" = "human face detected in an image or video frame", ["pii"];

    // Credentials: secrets and authentication.
    PASSWORD @ "credentials" = "password", ["secret"];
    SECURITY_QUESTION_ANSWER @ "credentials" = "account security question or answer"
        : "Knowledge-based challenge question or its answer used to recover or verify an account.",
        ["secret"];
    API_KEY @ "credentials" = "API key", ["secret"];
    AUTH_TOKEN @ "credentials" = "authentication token", ["secret"];
    PRIVATE_KEY @ "credentials" = "private cryptographic key", ["secret"];

    // Network: device and network identifiers.
    URL @ "network" = "URL or hyperlink", [];
    IP_ADDRESS @ "network" = "IP address", ["pii"];
    MAC_ADDRESS @ "network" = "MAC address", ["pii"];
    DEVICE_ID @ "network" = "device identifier", ["pii"];
    USERNAME @ "network" = "username or handle", ["pii"];

    // Organization: organizations and business entities.
    ORGANIZATION_NAME @ "organization" = "organization or company name", [];
    COMPANY_ID @ "organization" = "public company-registry identifier", [];
    DEPARTMENT_NAME @ "organization" = "department or business-unit name", [];
    FACILITY_NAME @ "organization" = "physical facility or location name", [];
    CASE_NUMBER @ "organization" = "case, matter, or docket number", [];
    INTERNAL_ID @ "organization" = "operator-defined internal identifier", [];
    OCCUPATION @ "organization" = "occupation or job title", [];
    PRODUCT @ "organization" = "product name", [];
    LOGO @ "organization" = "brand or organisation logo", [];

    // Contextual: dates, events, derived data, and the catch-all.
    DATE_TIME @ "contextual" = "date or time value", [];
    HANDWRITING @ "contextual" = "handwritten text", [];
    BARCODE @ "contextual" = "barcode or QR code", [];
    INDIVIDUAL_DATE @ "contextual" = "individual-associated date"
        : "Date directly relating to a natural person (birth, admission, discharge, death, or service date), as distinct from a bare calendar date such as an invoice or meeting date.",
        ["pii"];
    EVENT @ "contextual" = "named event reference", [];
    EDUCATION_RECORD @ "contextual" = "education record entry"
        : "Grade, transcript, disciplinary, or enrollment record for a person.",
        ["pii"];
    INFERENCE @ "contextual" = "profile inference"
        : "Model-derived characteristic inferred about a person to build a profile (preferences, psychological trends, predispositions, aptitudes), rather than a surface-level entity.",
        ["pii"];
    UNRESOLVED @ "contextual" = "unresolved entity"
        : "Sensitive entity whose specific type has not been resolved; a catch-all for detections that do not fit a more precise label.",
        [];
}

#[cfg(test)]
mod tests {
    use super::super::Category;
    use super::*;
    use crate::primitive::LanguageTag;

    #[test]
    fn well_known_built_ins_have_expected_ids_names_and_tags() {
        let en = LanguageTag::english();

        // The id is the constant's identifier, lowercased; the name is the
        // GLiNER-style natural-language phrase.
        assert_eq!(PAYMENT_CARD.id(), "payment_card");
        assert_eq!(PAYMENT_CARD.name(&en), "payment card number");
        assert_eq!(
            PAYMENT_CARD.category().map(Category::as_str),
            Some("financial")
        );
        assert!(PAYMENT_CARD.has_tag("pci"));
        assert!(PAYMENT_CARD.has_tag("pii"));

        assert_eq!(PERSON_NAME.id(), "person_name");
        assert_eq!(PERSON_NAME.name(&en), "person name");
        assert_eq!(
            PERSON_NAME.category().map(Category::as_str),
            Some("identity")
        );

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
    fn special_category_labels_ship_with_a_category() {
        let en = LanguageTag::english();

        // The special-category attributes GDPR Art. 9 covers group under
        // `protected_characteristic`; the category names *what the data is*,
        // never a regulatory regime (that mapping is a policy-layer concern).
        assert_eq!(POLITICAL_OPINION.id(), "political_opinion");
        assert_eq!(
            POLITICAL_OPINION.name(&en),
            "political opinion or affiliation"
        );

        for label in [
            &*POLITICAL_OPINION,
            &TRADE_UNION_MEMBERSHIP,
            &SEXUAL_ORIENTATION,
            &ETHNICITY,
            &RELIGION,
            &SEX_LIFE,
        ] {
            assert_eq!(
                label.category().map(Category::as_str),
                Some("protected_characteristic"),
            );
            // No regulation-citation tag leaked onto the taxonomy.
            assert!(!label.has_tag("article_9"));
        }

        assert_eq!(
            GENETIC_DATA.category().map(Category::as_str),
            Some("biometric")
        );
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
        assert_eq!(FAX_NUMBER.category().map(Category::as_str), Some("contact"));
        assert!(FAX_NUMBER.has_tag("pii"));

        assert_eq!(CERTIFICATE_NUMBER.id(), "certificate_number");
        // `certificate_number` is ambiguous, so it ships with a description.
        assert!(CERTIFICATE_NUMBER.description(&en).is_some());
    }

    #[test]
    fn geographic_granularity_labels_split_the_address() {
        let en = LanguageTag::english();

        // The address blob splits by granularity so a survivor set can keep
        // coarser components (state) while dropping finer ones (street). The
        // street line stays a contact identifier; town/state/country are
        // geographic.
        assert_eq!(
            STREET_ADDRESS.category().map(Category::as_str),
            Some("contact")
        );
        for label in [&*CITY, &STATE, &COUNTRY] {
            assert_eq!(label.category().map(Category::as_str), Some("geographic"));
        }
        assert_eq!(STREET_ADDRESS.id(), "street_address");
        assert!(STREET_ADDRESS.description(&en).is_some());
        // State and country survive coarser cuts, so they aren't tagged `pii`.
        assert!(!STATE.has_tag("pii"));
        assert!(!COUNTRY.has_tag("pii"));
    }

    #[test]
    fn built_ins_all_carry_a_category() {
        // Every shipped label has a category, so grouping never lands a
        // built-in in the uncategorized bucket.
        for label in BUILT_INS {
            assert!(
                label.category().is_some(),
                "built-in `{}` has no category",
                label.id(),
            );
        }
    }
}
