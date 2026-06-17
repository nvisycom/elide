//! Universal patterns — apply regardless of jurisdiction.

use crate::{__shipped_pattern as shipped_pattern, Regex};

shipped_pattern!(
    /// Email address (RFC-loose).
    fn email from "../../../assets/patterns/world/contact/email.toml"
);
shipped_pattern!(
    /// International phone numbers.
    fn phone from "../../../assets/patterns/world/contact/phone.toml"
);
shipped_pattern!(
    /// URLs (HTTP/HTTPS/FTP).
    fn url from "../../../assets/patterns/world/contact/url.toml"
);

shipped_pattern!(
    /// AWS access key IDs.
    fn aws_key from "../../../assets/patterns/world/credentials/aws_key.toml"
);
shipped_pattern!(
    /// Heuristic generic API key.
    fn generic_api_key from "../../../assets/patterns/world/credentials/generic_api_key.toml"
);
shipped_pattern!(
    /// GitHub personal access tokens.
    fn github_token from "../../../assets/patterns/world/credentials/github_token.toml"
);
shipped_pattern!(
    /// PEM-formatted private keys.
    fn private_key from "../../../assets/patterns/world/credentials/private_key.toml"
);
shipped_pattern!(
    /// Stripe live/test secret keys.
    fn stripe_key from "../../../assets/patterns/world/credentials/stripe_key.toml"
);

shipped_pattern!(
    /// Bitcoin (legacy + bech32) addresses.
    fn bitcoin_address from "../../../assets/patterns/world/finance/bitcoin_address.toml"
);
shipped_pattern!(
    /// Credit-card numbers, Luhn-validated.
    fn credit_card from "../../../assets/patterns/world/finance/credit_card.toml"
);
shipped_pattern!(
    /// Ethereum addresses.
    fn ethereum_address from "../../../assets/patterns/world/finance/ethereum_address.toml"
);
shipped_pattern!(
    /// International Bank Account Numbers.
    fn iban from "../../../assets/patterns/world/finance/iban.toml"
);
shipped_pattern!(
    /// SWIFT / BIC codes.
    fn swift_code from "../../../assets/patterns/world/finance/swift_code.toml"
);

shipped_pattern!(
    /// IPv4 addresses.
    fn ipv4 from "../../../assets/patterns/world/network/ipv4.toml"
);
shipped_pattern!(
    /// IPv6 addresses.
    fn ipv6 from "../../../assets/patterns/world/network/ipv6.toml"
);
shipped_pattern!(
    /// MAC (Ethernet) addresses.
    fn mac_address from "../../../assets/patterns/world/network/mac_address.toml"
);

shipped_pattern!(
    /// Date of birth in common written formats.
    fn date_of_birth from "../../../assets/patterns/world/personal/date_of_birth.toml"
);
shipped_pattern!(
    /// Date + time stamps in ISO-like formats.
    fn datetime from "../../../assets/patterns/world/personal/datetime.toml"
);

/// Every world-scoped built-in pattern.
#[must_use]
pub fn all() -> Vec<Regex> {
    vec![
        email(),
        phone(),
        url(),
        aws_key(),
        generic_api_key(),
        github_token(),
        private_key(),
        stripe_key(),
        bitcoin_address(),
        credit_card(),
        ethereum_address(),
        iban(),
        swift_code(),
        ipv4(),
        ipv6(),
        mac_address(),
        date_of_birth(),
        datetime(),
    ]
}
