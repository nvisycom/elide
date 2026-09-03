//! Financial category: currency codes and monetary amounts. The structured
//! kinds (IBAN, payment card, bank account/routing, SWIFT, crypto address,
//! card security code, card expiry) pattern-preserve their original and don't
//! go through this module.

use fake::Fake;
use fake::faker::currency::raw as currency;
use fake::rand::RngExt;

use super::Original;
use super::dispatch::fan_locale;
use crate::locale::Locale;

pub(super) fn currency_code<R: RngExt + ?Sized>(locale: Locale, rng: &mut R) -> String {
    fan_locale!(locale, rng, currency::CurrencyCode)
}

/// Locale-aware monetary amount preserving the original's *magnitude*: a
/// millions figure is replaced by another millions figure, not a handful of
/// dollars, since the order of magnitude is obvious from context. SI/UK locales
/// format `1234.56`; continental EU locales `1234,56`.
///
/// The whole part gets the same number of digits as the original (falling back
/// to a broad range when it carries no digits); a two-digit fraction is always
/// appended.
pub(super) fn monetary_amount<R: RngExt + ?Sized>(
    original: &Original<'_>,
    locale: Locale,
    rng: &mut R,
) -> String {
    // Same digit count as the original's whole part, default 1..=6 digits.
    // Cap the width so 10^d stays within u64 (its max is ~1.8e19, 20 digits).
    let digits = original.digit_magnitude().unwrap_or(0).min(18);
    let whole: u64 = if original.whole_is_zero() {
        // A sub-unit or zero amount ("0.50", "$0.00"): keep the zero whole part,
        // inflating it to 1..9 would misstate the magnitude.
        0
    } else if digits == 0 {
        (0..1_000_000u64).fake_with_rng(rng)
    } else {
        // A `digits`-wide number: [10^(d-1), 10^d), so the leading digit is
        // non-zero and the width matches.
        let low = 10u64.pow(digits - 1);
        let high = low * 10;
        (low..high).fake_with_rng(rng)
    };
    // A zero-whole amount keeps a nonzero fraction so it stays a plausible
    // sub-unit value (0.50 -> 0.NN, never 0.00); otherwise any fraction is fine.
    let frac: u8 = if whole == 0 {
        (1..100u8).fake_with_rng(rng)
    } else {
        (0..100u8).fake_with_rng(rng)
    };
    let sep = decimal_separator(locale);
    format!("{whole}{sep}{frac:02}")
}

fn decimal_separator(locale: Locale) -> char {
    match locale {
        Locale::DeDe
        | Locale::NlNl
        | Locale::FrFr
        | Locale::ItIt
        | Locale::PtPt
        | Locale::PtBr
        | Locale::TrTr => ',',
        _ => '.',
    }
}

#[cfg(test)]
mod tests {
    use fake::rand::SeedableRng;
    use fake::rand::rngs::SmallRng;

    use super::*;

    fn whole_part(amount: &str) -> &str {
        amount.rsplit_once(['.', ',']).map_or(amount, |(w, _)| w)
    }

    #[test]
    fn monetary_amount_uses_locale_decimal_separator() {
        let orig = Original::new("100.00");
        for locale in [Locale::DeDe, Locale::TrTr] {
            let mut rng = SmallRng::seed_from_u64(1);
            assert!(
                monetary_amount(&orig, locale, &mut rng).contains(','),
                "{locale:?} should use a comma"
            );
        }
        let mut rng = SmallRng::seed_from_u64(1);
        assert!(monetary_amount(&orig, Locale::En, &mut rng).contains('.'));
    }

    #[test]
    fn a_sub_unit_amount_keeps_a_zero_whole_part() {
        // A "$0.50" must not inflate to "$3.50": the whole part stays 0, and the
        // fraction stays nonzero so it is still a plausible small amount.
        for seed in 0..50u64 {
            let mut rng = SmallRng::seed_from_u64(seed);
            let out = monetary_amount(&Original::new("0.50"), Locale::En, &mut rng);
            assert_eq!(whole_part(&out), "0", "{out} inflated the whole part");
            assert_ne!(out, "0.00", "{out} zeroed a sub-unit amount");
        }
    }

    #[test]
    fn monetary_amount_preserves_the_originals_magnitude() {
        // A millions figure stays millions; a two-digit figure stays two-digit.
        for (original, whole_digits) in [("$2,000,000.00", 7), ("42.50", 2), ("1.234.567,89 €", 7)]
        {
            for seed in 0..30u64 {
                let mut rng = SmallRng::seed_from_u64(seed);
                let out = monetary_amount(&Original::new(original), Locale::En, &mut rng);
                assert_eq!(
                    whole_part(&out).len(),
                    whole_digits,
                    "{original} -> {out} changed magnitude"
                );
            }
        }
    }
}
