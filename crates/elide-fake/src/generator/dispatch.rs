//! Locale-dispatch macro for fake-rs fakers.
//!
//! Each `fake` locale (`EN`, `FR_FR`, …) implements a different set
//! of `*GenFn` trait gates per faker, so a generic function bounded
//! on [`fake::locales::Data`] can't reach every faker. The
//! [`fan_locale!`] macro fans a single faker constructor across all
//! 14 [`Locale`] variants at the call site.
//!
//! The macro is self-contained — it imports `fake::Fake` and every
//! locale constant it needs into the expansion's scope, so callers
//! don't have to.
//!
//! [`Locale`]: crate::locale::Locale

/// Invoke `$faker(locale)` for each [`Locale`] variant and call
/// `.fake_with_rng($rng)`.
///
/// [`Locale`]: crate::locale::Locale
macro_rules! fan_locale {
    ($locale:expr, $rng:expr, $faker:expr) => {{
        use ::fake::Fake as _;
        use ::fake::locales::{
            AR_SA, CY_GB, DE_DE, EN, FA_IR, FR_FR, IT_IT, JA_JP, NL_NL, PT_BR, PT_PT, TR_TR, ZH_CN,
            ZH_TW,
        };
        match $locale {
            $crate::locale::Locale::En => $faker(EN).fake_with_rng($rng),
            $crate::locale::Locale::FrFr => $faker(FR_FR).fake_with_rng($rng),
            $crate::locale::Locale::JaJp => $faker(JA_JP).fake_with_rng($rng),
            $crate::locale::Locale::ZhCn => $faker(ZH_CN).fake_with_rng($rng),
            $crate::locale::Locale::ZhTw => $faker(ZH_TW).fake_with_rng($rng),
            $crate::locale::Locale::DeDe => $faker(DE_DE).fake_with_rng($rng),
            $crate::locale::Locale::ItIt => $faker(IT_IT).fake_with_rng($rng),
            $crate::locale::Locale::PtBr => $faker(PT_BR).fake_with_rng($rng),
            $crate::locale::Locale::PtPt => $faker(PT_PT).fake_with_rng($rng),
            $crate::locale::Locale::NlNl => $faker(NL_NL).fake_with_rng($rng),
            $crate::locale::Locale::TrTr => $faker(TR_TR).fake_with_rng($rng),
            $crate::locale::Locale::ArSa => $faker(AR_SA).fake_with_rng($rng),
            $crate::locale::Locale::FaIr => $faker(FA_IR).fake_with_rng($rng),
            $crate::locale::Locale::CyGb => $faker(CY_GB).fake_with_rng($rng),
        }
    }};
}

pub(crate) use fan_locale;
