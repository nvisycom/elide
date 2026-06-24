//! Free-form device/credential generators: passwords, API tokens,
//! device UUIDs. Structured kinds (IpAddress, MacAddress)
//! pattern-preserve their original and don't go through this
//! module.

use fake::Fake;
use fake::faker::internet::raw as internet;
use fake::locales::{
    AR_SA, CY_GB, DE_DE, EN, FA_IR, FR_FR, IT_IT, JA_JP, NL_NL, PT_BR, PT_PT, TR_TR, ZH_CN, ZH_TW,
};
use fake::rand::RngExt;
use uuid::Uuid;

use crate::locale::Locale;

/// `internet::Password` takes both a locale and a length range, so
/// it can't go through the `fan_locale!` macro (which assumes a
/// one-arg faker constructor). Manual per-locale dispatch instead.
pub(super) fn password<R: RngExt + ?Sized>(locale: Locale, rng: &mut R) -> String {
    let range = 12..24;
    match locale {
        Locale::En => internet::Password(EN, range).fake_with_rng(rng),
        Locale::FrFr => internet::Password(FR_FR, range).fake_with_rng(rng),
        Locale::JaJp => internet::Password(JA_JP, range).fake_with_rng(rng),
        Locale::ZhCn => internet::Password(ZH_CN, range).fake_with_rng(rng),
        Locale::ZhTw => internet::Password(ZH_TW, range).fake_with_rng(rng),
        Locale::DeDe => internet::Password(DE_DE, range).fake_with_rng(rng),
        Locale::ItIt => internet::Password(IT_IT, range).fake_with_rng(rng),
        Locale::PtBr => internet::Password(PT_BR, range).fake_with_rng(rng),
        Locale::PtPt => internet::Password(PT_PT, range).fake_with_rng(rng),
        Locale::NlNl => internet::Password(NL_NL, range).fake_with_rng(rng),
        Locale::TrTr => internet::Password(TR_TR, range).fake_with_rng(rng),
        Locale::ArSa => internet::Password(AR_SA, range).fake_with_rng(rng),
        Locale::FaIr => internet::Password(FA_IR, range).fake_with_rng(rng),
        Locale::CyGb => internet::Password(CY_GB, range).fake_with_rng(rng),
    }
}

/// Random 32-char lowercase hex — looks like an API token without
/// claiming any particular provider's shape.
pub(super) fn api_key<R: RngExt + ?Sized>(rng: &mut R) -> String {
    hex_chars(32, rng)
}

/// Random 48-char lowercase hex — slightly longer than `api_key`
/// so the two are visually distinguishable.
pub(super) fn auth_token<R: RngExt + ?Sized>(rng: &mut R) -> String {
    hex_chars(48, rng)
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

fn hex_chars<R: RngExt + ?Sized>(len: usize, rng: &mut R) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(len);
    for _ in 0..len {
        let n: u32 = (0..16u32).fake_with_rng(rng);
        out.push(HEX[n as usize] as char);
    }
    out
}
