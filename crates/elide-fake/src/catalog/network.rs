//! Network category: usernames, URLs, and device identifiers. The structured
//! kinds (IP address, MAC address) pattern-preserve their original and don't go
//! through this module.

use fake::Fake;
use fake::faker::internet::raw as internet;
use fake::rand::RngExt;
use uuid::Uuid;

use super::dispatch::fan_locale;
use crate::locale::Locale;

pub(super) fn username<R: RngExt + ?Sized>(locale: Locale, rng: &mut R) -> String {
    fan_locale!(locale, rng, internet::Username)
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

/// UUIDv4 in canonical hex-with-hyphens form.
pub(super) fn device_id<R: RngExt + ?Sized>(rng: &mut R) -> String {
    let mut bytes = [0u8; 16];
    for b in &mut bytes {
        let n: u32 = (0..256u32).fake_with_rng(rng);
        *b = n as u8;
    }
    Uuid::from_bytes(bytes).to_string()
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
