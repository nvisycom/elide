//! People and categorical identifiers: names, organisations, jobs,
//! and short categorical pick-from-list values (gender, language,
//! nationality, citizenship).

use fake::Fake;
use fake::faker::address::raw as address;
use fake::faker::company::raw as company;
use fake::faker::internet::raw as internet;
use fake::faker::job::raw as job;
use fake::faker::name::raw as name;
use fake::rand::RngExt;

use super::dispatch::fan_locale;
use crate::locale::Locale;

pub(super) fn person_name<R: RngExt + ?Sized>(locale: Locale, rng: &mut R) -> String {
    fan_locale!(locale, rng, name::Name)
}

pub(super) fn organization_name<R: RngExt + ?Sized>(locale: Locale, rng: &mut R) -> String {
    fan_locale!(locale, rng, company::CompanyName)
}

pub(super) fn occupation<R: RngExt + ?Sized>(locale: Locale, rng: &mut R) -> String {
    fan_locale!(locale, rng, job::Position)
}

pub(super) fn username<R: RngExt + ?Sized>(locale: Locale, rng: &mut R) -> String {
    fan_locale!(locale, rng, internet::Username)
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
