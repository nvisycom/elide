//! Demographic category: personal attributes, age, gender, nationality,
//! citizenship, and spoken language.

use fake::Fake;
use fake::faker::address::raw as address;
use fake::rand::RngExt;

use super::Original;
use super::dispatch::fan_locale;
use crate::locale::Locale;

/// Fake an age within the *same life-stage band* as the original, so a
/// 90-year-old is replaced by another elderly age and a toddler by another
/// toddler. Replacing across bands would both fail to anonymize (the band is
/// obvious from context) and read as wrong.
///
/// The original is expected to be a bare year count. When it isn't a number the
/// band is unknown, so this falls back to a broad adult range.
pub(super) fn age<R: RngExt + ?Sized>(original: &Original<'_>, rng: &mut R) -> String {
    let band = original.number::<u8>().map(age_band).unwrap_or((18, 64));
    let years: u8 = (band.0..=band.1).fake_with_rng(rng);
    years.to_string()
}

/// The inclusive `[low, high]` life-stage band an age falls in.
fn age_band(years: u8) -> (u8, u8) {
    match years {
        0..=2 => (0, 2),     // infant / toddler
        3..=12 => (3, 12),   // child
        13..=17 => (13, 17), // teenager
        18..=29 => (18, 29), // young adult
        30..=44 => (30, 44), // adult
        45..=64 => (45, 64), // middle-aged
        65..=79 => (65, 79), // senior
        _ => (80, 105),      // elderly
    }
}

pub(super) fn gender<R: RngExt + ?Sized>(locale: Locale, rng: &mut R) -> String {
    let options = gender_options(locale);
    pick(options, rng).to_owned()
}

pub(super) fn language<R: RngExt + ?Sized>(rng: &mut R) -> String {
    // BCP-47 tags are locale-invariant identifiers.
    const TAGS: &[&str] = &[
        "en", "fr", "de", "ja", "zh", "es", "it", "pt", "ar", "ru", "nl", "tr", "ko",
    ];
    pick(TAGS, rng).to_owned()
}

pub(super) fn nationality<R: RngExt + ?Sized>(locale: Locale, rng: &mut R) -> String {
    fan_locale!(locale, rng, address::CountryName)
}

pub(super) fn citizenship<R: RngExt + ?Sized>(locale: Locale, rng: &mut R) -> String {
    fan_locale!(locale, rng, address::CountryName)
}

/// Per-locale gender label sets. English is the fallback for
/// locales without an explicit list.
fn gender_options(locale: Locale) -> &'static [&'static str] {
    match locale {
        Locale::FrFr => &["féminin", "masculin", "non-binaire", "autre"],
        Locale::DeDe => &["weiblich", "männlich", "nicht-binär", "andere"],
        Locale::ItIt => &["femminile", "maschile", "non-binario", "altro"],
        Locale::PtBr | Locale::PtPt => &["feminino", "masculino", "não-binário", "outro"],
        Locale::NlNl => &["vrouwelijk", "mannelijk", "non-binair", "anders"],
        Locale::JaJp => &["女性", "男性", "その他"],
        Locale::ZhCn | Locale::ZhTw => &["女性", "男性", "其他"],
        _ => &["female", "male", "non-binary", "other", "prefer not to say"],
    }
}

fn pick<'a, R: RngExt + ?Sized>(options: &'a [&'a str], rng: &mut R) -> &'a str {
    let i: usize = (0..options.len()).fake_with_rng(rng);
    options[i]
}

#[cfg(test)]
mod tests {
    use fake::rand::SeedableRng;
    use fake::rand::rngs::SmallRng;

    use super::*;

    #[test]
    fn age_stays_in_the_originals_life_stage_band() {
        // An elderly age is replaced by another elderly age, never a toddler; a
        // toddler by another toddler. The band is obvious from context, so a
        // cross-band swap would read as wrong.
        for (original, low, high) in [("90", 80, 105), ("2", 0, 2), ("40", 30, 44)] {
            for seed in 0..50u64 {
                let mut rng = SmallRng::seed_from_u64(seed);
                let faked: u8 = age(&Original::new(original), &mut rng)
                    .parse()
                    .expect("age is a number");
                assert!(
                    (low..=high).contains(&faked),
                    "age {original} -> {faked} left band [{low}, {high}]"
                );
            }
        }
    }

    #[test]
    fn a_non_numeric_age_falls_back_to_an_adult_range() {
        let mut rng = SmallRng::seed_from_u64(3);
        let faked: u8 = age(&Original::new("ninety"), &mut rng)
            .parse()
            .expect("age is a number");
        assert!((18..=64).contains(&faked));
    }
}
