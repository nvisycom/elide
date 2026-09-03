//! Free-form contact generators: street address composition, URL
//! synthesis. The structured kinds (EmailAddress, PhoneNumber,
//! PostalCode, Coordinates, LicensePlate) pattern-preserve their
//! original and don't go through this module.

use fake::faker::address::raw as address;
use fake::faker::internet::raw as internet;
use fake::rand::RngExt;

use super::dispatch::fan_locale;
use crate::locale::Locale;

pub(super) fn street_address<R: RngExt + ?Sized>(locale: Locale, rng: &mut R) -> String {
    let building: String = fan_locale!(locale, rng, address::BuildingNumber);
    let street: String = fan_locale!(locale, rng, address::StreetName);
    let city: String = fan_locale!(locale, rng, address::CityName);
    match locale {
        // CJK addresses go big-to-small (prefecture → ward → block →
        // building) and don't concatenate street + building the way
        // Latin-script ones do. This is "less wrong than English
        // ordering," not a faithful rendering.
        Locale::JaJp | Locale::ZhCn | Locale::ZhTw => format!("{city}{street}{building}"),
        _ => format!("{building} {street}, {city}"),
    }
}

pub(super) fn url<R: RngExt + ?Sized>(locale: Locale, rng: &mut R) -> String {
    let user: String = fan_locale!(locale, rng, internet::Username);
    let domain: String = fan_locale!(locale, rng, internet::DomainSuffix);
    let host = sanitise_hostname_label(&user);
    let host = if host.is_empty() {
        "site"
    } else {
        host.as_str()
    };
    format!("https://www.{host}.{domain}")
}

/// Strip characters that aren't valid in a DNS label
/// (RFC 1035: ASCII letters, digits, and hyphens), and trim leading
/// or trailing hyphens. Returns lowercase output.
fn sanitise_hostname_label(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' {
            out.push(ch.to_ascii_lowercase());
        }
    }
    out.trim_matches('-').to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_non_dns_characters() {
        assert_eq!(sanitise_hostname_label("Ali_ce"), "alice");
        assert_eq!(sanitise_hostname_label("Bob.Smith"), "bobsmith");
        assert_eq!(sanitise_hostname_label("-mid-"), "mid");
    }

    #[test]
    fn handles_empty_after_strip() {
        assert!(sanitise_hostname_label("___").is_empty());
    }
}
