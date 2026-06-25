//! Built-in [`Label`] constants.
//!
//! Each constant carries a category tag (`personal_identity`,
//! `financial`, …) plus cross-cutting tags where applicable (`pii`,
//! `phi`, `pci`). Selectors can match by label name *or* by tag without
//! the workspace modelling categories as a separate enum.
//!
//! The `BUILT_INS` slice indexes every constant for the
//! [`LabelCatalog::with_builtins`] constructor; the constants themselves
//! are public and reachable by name (e.g. `builtins::PERSON_NAME`).
//!
//! [`LabelCatalog::with_builtins`]: super::LabelCatalog::with_builtins

use std::sync::LazyLock;

use super::Label;

macro_rules! label {
    ($vis:vis $ident:ident, $name:literal, $desc:literal, [ $($tag:literal),* $(,)? ]) => {
        #[doc = $desc]
        $vis static $ident: LazyLock<Label> = LazyLock::new(|| {
            Label::from_static($name, Some($desc), &[$($tag),*])
        });
    };
}

label!(pub PERSON_NAME, "person_name","Person name (full, first, or last).", ["personal_identity", "pii"]);
label!(pub DATE_OF_BIRTH, "date_of_birth","Date of birth.", ["personal_identity", "pii"]);
label!(pub GOVERNMENT_ID, "government_id","Government-issued identification number (SSN, SIN, Aadhaar, national ID, etc.).", ["personal_identity", "pii"]);
label!(pub TAX_ID, "tax_id","Tax identification number (ITIN, EIN, TIN, etc.).", ["personal_identity", "pii"]);
label!(pub DRIVERS_LICENSE, "drivers_license","Driver's license number.", ["personal_identity", "pii"]);
label!(pub PASSPORT_NUMBER, "passport_number","Passport number.", ["personal_identity", "pii"]);
label!(pub NATIONAL_INSURANCE_NUMBER, "national_insurance_number","National insurance or social-security equivalent (NI, BSN, AHVN, etc.).", ["personal_identity", "pii"]);
label!(pub VEHICLE_ID, "vehicle_id","Vehicle identification number (VIN).", ["personal_identity"]);
label!(pub LICENSE_PLATE, "license_plate","License plate number.", ["personal_identity"]);
label!(pub EMAIL_ADDRESS, "email_address","Email address.", ["contact_info", "pii"]);
label!(pub PHONE_NUMBER, "phone_number","Phone number.", ["contact_info", "pii"]);
label!(pub ADDRESS, "address","Physical or mailing address.", ["contact_info", "pii"]);
label!(pub POSTAL_CODE, "postal_code","Postal or ZIP code.", ["contact_info"]);
label!(pub URL, "url","URL or hyperlink.", ["contact_info"]);
label!(pub AGE, "age","Age value.", ["demographic", "pii"]);
label!(pub GENDER, "gender","Gender identity.", ["demographic", "pii"]);
label!(pub ETHNICITY, "ethnicity","Racial or ethnic background.", ["demographic", "pii"]);
label!(pub RELIGION, "religion","Religious affiliation.", ["demographic", "pii"]);
label!(pub NATIONALITY, "nationality","Nationality.", ["demographic", "pii"]);
label!(pub CITIZENSHIP, "citizenship","Citizenship status.", ["demographic", "pii"]);
label!(pub LANGUAGE, "language","Language or dialect spoken.", ["demographic"]);
label!(pub PAYMENT_CARD, "payment_card","Payment card number (credit or debit).", ["financial", "pci", "pii"]);
label!(pub CARD_SECURITY_CODE, "card_security_code","Payment card security code (CVV/CVC).", ["financial", "pci"]);
label!(pub CARD_EXPIRY, "card_expiry","Payment card expiration date.", ["financial", "pci"]);
label!(pub BANK_ACCOUNT, "bank_account","Bank account number.", ["financial", "pii"]);
label!(pub BANK_ROUTING, "bank_routing","Bank routing or transit number.", ["financial"]);
label!(pub IBAN, "iban","International Bank Account Number (IBAN).", ["financial", "pii"]);
label!(pub SWIFT_CODE, "swift_code","SWIFT/BIC code.", ["financial"]);
label!(pub CRYPTO_ADDRESS, "crypto_address","Cryptocurrency wallet address.", ["financial", "pii"]);
label!(pub CURRENCY, "currency","Currency code or symbol.", ["financial"]);
label!(pub AMOUNT, "amount","Monetary amount.", ["financial"]);
label!(pub MEDICAL_ID, "medical_id","Medical record number.", ["health", "phi", "pii"]);
label!(pub INSURANCE_ID, "insurance_id","Health insurance identifier.", ["health", "phi", "pii"]);
label!(pub PRESCRIPTION_ID, "prescription_id","Prescription identifier or medication regimen.", ["health", "phi"]);
label!(pub DIAGNOSIS, "diagnosis","Medical diagnosis or condition.", ["health", "phi"]);
label!(pub MEDICATION, "medication","Medication name.", ["health", "phi"]);
label!(pub FINGERPRINT, "fingerprint","Fingerprint biometric data.", ["biometric", "pii"]);
label!(pub VOICEPRINT, "voiceprint","Voiceprint biometric data.", ["biometric", "pii"]);
label!(pub RETINA_SCAN, "retina_scan","Retina scan biometric data.", ["biometric", "pii"]);
label!(pub FACIAL_GEOMETRY, "facial_geometry","Facial geometry biometric data.", ["biometric", "pii"]);
label!(pub PASSWORD, "password","Password.", ["credentials", "secret"]);
label!(pub API_KEY, "api_key","API key.", ["credentials", "secret"]);
label!(pub AUTH_TOKEN, "auth_token","Authentication token (OAuth, JWT, session token).", ["credentials", "secret"]);
label!(pub PRIVATE_KEY, "private_key","Private cryptographic key.", ["credentials", "secret"]);
label!(pub IP_ADDRESS, "ip_address","IP address (v4 or v6).", ["network_identifier", "pii"]);
label!(pub MAC_ADDRESS, "mac_address","MAC address.", ["network_identifier", "pii"]);
label!(pub DEVICE_ID, "device_id","Device identifier (IMEI, UDID, etc.).", ["network_identifier", "pii"]);
label!(pub USERNAME, "username","Username or handle.", ["network_identifier", "pii"]);
label!(pub COORDINATES, "coordinates","GPS coordinates (latitude/longitude).", ["location", "pii"]);
label!(pub GEOLOCATION_METADATA, "geolocation_metadata","Geolocation metadata.", ["location", "pii"]);
label!(pub FACE, "face","Human face detected in an image or video frame.", ["visual", "pii"]);
label!(pub HANDWRITING, "handwriting","Handwritten text.", ["visual"]);
label!(pub SIGNATURE, "signature","Handwritten signature.", ["visual", "pii"]);
label!(pub LOGO, "logo","Brand or organisation logo.", ["visual"]);
label!(pub BARCODE, "barcode","Barcode or QR code.", ["visual"]);
label!(pub ORGANIZATION_NAME, "organization_name","Organization or company name.", ["organization"]);
label!(pub COMPANY_ID, "company_id","Public company-registry identifier (Handelsregisternummer, Companies House number, etc.).", ["organization"]);
label!(pub DEPARTMENT_NAME, "department_name","Department or business-unit name.", ["organization"]);
label!(pub FACILITY_NAME, "facility_name","Physical facility or location name.", ["organization"]);
label!(pub CASE_NUMBER, "case_number","Case, matter, or docket number.", ["organization"]);
label!(pub INTERNAL_ID, "internal_id","Operator-defined internal identifier.", ["organization"]);
label!(pub DATE_TIME, "date_time","Date or time value.", ["temporal"]);
label!(pub EVENT, "event","Named event reference.", ["temporal"]);
label!(pub OCCUPATION, "occupation","Occupation or job title.", ["organization"]);
label!(pub PRODUCT, "product","Product name.", ["organization"]);
label!(pub QUANTITY, "quantity","Numerical quantity.", ["quantity"]);
label!(pub UNRESOLVED, "unresolved","Entity kind not yet identified.", ["unresolved"]);

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
    use super::*;

    #[test]
    fn well_known_built_ins_have_expected_tags() {
        assert_eq!(PAYMENT_CARD.name(), "payment_card");
        assert!(PAYMENT_CARD.has_tag("financial"));
        assert!(PAYMENT_CARD.has_tag("pci"));
        assert!(PAYMENT_CARD.has_tag("pii"));
        assert_eq!(PERSON_NAME.name(), "person_name");
        assert!(PERSON_NAME.has_tag("personal_identity"));
    }
}
