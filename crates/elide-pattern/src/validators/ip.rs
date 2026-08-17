//! IP-address validators backed by the standard library's parsers.
//!
//! IPv6 in particular has enough compression forms (`::`, embedded IPv4,
//! mixed splits) that a single regex either misses cases or over-matches. A
//! permissive regex captures an address-shaped token and these validators
//! confirm it with [`std::net`], the reference parser — the same
//! permissive-match-then-validate approach the checksum validators use.

use std::net::{Ipv4Addr, Ipv6Addr};

/// Return `true` if `value` is a valid IPv4 address, optionally with a `/N`
/// CIDR prefix (`0..=32`). The address itself is parsed by [`Ipv4Addr`].
pub fn ipv4(value: &str) -> bool {
    let (addr, prefix) = split_cidr(value.trim());
    if let Some(bits) = prefix
        && bits > 32
    {
        return false;
    }
    addr.parse::<Ipv4Addr>().is_ok()
}

/// Return `true` if `value` is a valid IPv6 address, optionally with a `%zone`
/// scope id and/or a `/N` CIDR prefix (`0..=128`). The address itself is
/// parsed by [`Ipv6Addr`], which accepts every canonical compression form.
pub fn ipv6(value: &str) -> bool {
    let (addr, prefix) = split_cidr(value.trim());
    if let Some(bits) = prefix
        && bits > 128
    {
        return false;
    }
    // Drop a `%zone` scope id before parsing: it is valid in text but not
    // accepted by `Ipv6Addr::from_str`.
    let addr = addr.split_once('%').map_or(addr, |(head, _)| head);
    addr.parse::<Ipv6Addr>().is_ok()
}

/// Split a trailing `/N` CIDR suffix off, returning the address part and the
/// parsed prefix length (if any). A malformed prefix yields `Some(u16::MAX)`
/// so the caller's range check rejects it.
fn split_cidr(value: &str) -> (&str, Option<u16>) {
    match value.split_once('/') {
        Some((addr, bits)) => (addr, Some(bits.parse().unwrap_or(u16::MAX))),
        None => (value, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipv6_accepts_all_compression_forms() {
        for addr in [
            "2001:db8::8a2e:370:7334",
            "2001:0db8:85a3:0000:0000:8a2e:0370:7334",
            "2001:db8::",
            "::1",
            "::",
            "fe80::1%eth0",
            "::ffff:192.0.2.1",
            "2001:db8::8a2e:370:7334/64",
        ] {
            assert!(ipv6(addr), "{addr} should be a valid IPv6 address");
        }
    }

    #[test]
    fn ipv6_rejects_non_addresses() {
        for addr in [
            "2001:db8:::1",      // triple colon
            "gggg::1",           // non-hex
            "1:2:3:4:5:6:7:8:9", // too many groups
            "2001:db8::/129",    // prefix out of range
            "not an address",
        ] {
            assert!(!ipv6(addr), "{addr} should be rejected");
        }
    }

    #[test]
    fn ipv4_accepts_and_rejects() {
        assert!(ipv4("192.0.2.44"));
        assert!(ipv4("10.0.0.1/8"));
        assert!(!ipv4("256.0.0.1"));
        assert!(!ipv4("192.0.2.44/33"));
        assert!(!ipv4("1.2.3"));
    }
}
