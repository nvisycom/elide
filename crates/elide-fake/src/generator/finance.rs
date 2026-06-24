//! Free-form financial generators: currency code, monetary
//! amounts, quantities. The structured kinds (IBAN, PaymentCard,
//! BankAccount, BankRouting, SwiftCode, CardSecurityCode,
//! CardExpiry) all pattern-preserve their original and don't go
//! through this module.

use fake::Fake;
use fake::faker::currency::raw as currency;
use fake::rand::RngExt;

use super::dispatch::fan_locale;
use crate::locale::Locale;

pub(super) fn currency_code<R: RngExt + ?Sized>(locale: Locale, rng: &mut R) -> String {
    fan_locale!(locale, rng, currency::CurrencyCode)
}

/// Locale-aware monetary amount. SI/UK locales use `1234.56`;
/// continental EU locales use `1234,56`.
pub(super) fn amount<R: RngExt + ?Sized>(locale: Locale, rng: &mut R) -> String {
    let cents: u32 = (0..10_000_000u32).fake_with_rng(rng);
    let whole = cents / 100;
    let frac = cents % 100;
    let sep = decimal_separator(locale);
    format!("{whole}{sep}{frac:02}")
}

/// Plain integer in `0..=10_000`.
pub(super) fn quantity<R: RngExt + ?Sized>(rng: &mut R) -> String {
    let n: u32 = (0..=10_000u32).fake_with_rng(rng);
    n.to_string()
}

fn decimal_separator(locale: Locale) -> char {
    match locale {
        Locale::DeDe | Locale::NlNl | Locale::FrFr | Locale::ItIt | Locale::PtPt | Locale::PtBr => {
            ','
        }
        _ => '.',
    }
}

#[cfg(test)]
mod tests {
    use fake::rand::SeedableRng;
    use fake::rand::rngs::SmallRng;

    use super::*;

    #[test]
    fn amount_uses_locale_decimal_separator() {
        let mut rng = SmallRng::seed_from_u64(1);
        assert!(amount(Locale::DeDe, &mut rng).contains(','));
        let mut rng = SmallRng::seed_from_u64(1);
        assert!(amount(Locale::En, &mut rng).contains('.'));
    }
}
