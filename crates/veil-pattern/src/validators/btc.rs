//! Bitcoin legacy-address (Base58Check) checksum validator.
//!
//! Validates P2PKH (`1…`) and P2SH (`3…`) addresses by decoding
//! the Base58 payload and verifying its trailing four-byte
//! double-SHA256 checksum. Bech32 / Bech32m addresses (`bc1…`)
//! are not handled here — those use a different polynomial check.

/// Return `true` if `value` is a structurally valid Base58Check
/// Bitcoin address.
///
/// Accepts P2PKH (version byte `0x00`, `1…`) and P2SH
/// (version byte `0x05`, `3…`) on mainnet. Rejects mismatched
/// version bytes, broken Base58, and bad checksums.
pub fn btc(value: &str) -> bool {
    match bs58::decode(value.trim()).with_check(None).into_vec() {
        Ok(bytes) if bytes.len() == 21 => matches!(bytes[0], 0x00 | 0x05),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_known_p2pkh() {
        // Satoshi's genesis-block coinbase address.
        assert!(btc("1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa"));
    }

    #[test]
    fn accepts_known_p2sh() {
        assert!(btc("3J98t1WpEZ73CNmQviecrnyiWrnqRhWNLy"));
    }

    #[test]
    fn rejects_bad_checksum() {
        // Final char flipped.
        assert!(!btc("1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNb"));
    }

    #[test]
    fn rejects_non_base58() {
        assert!(!btc("1A1zP1eP5QGefi2DMPTfTL5SLmv7Divf0a"));
        assert!(!btc(""));
    }

    #[test]
    fn rejects_unknown_version() {
        // Bitcoin testnet P2PKH (version byte `0x6f`).
        assert!(!btc("mipcBbFg9gMiCh81Kj8tqqdgoZub1ZJRfn"));
    }
}
