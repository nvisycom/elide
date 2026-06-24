//! Internal locale picker: maps BCP-47 tags to `fake` crate locale
//! variants.

use elide_core::primitive::LanguageTag;

/// Locale selector for the fake-data generator. Maps to one of the
/// `fake` crate's locale modules. Internal — callers express locale
/// via [`LanguageTag`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub(crate) enum Locale {
    #[default]
    En,
    FrFr,
    JaJp,
    ZhCn,
    ZhTw,
    DeDe,
    ItIt,
    PtBr,
    PtPt,
    NlNl,
    TrTr,
    ArSa,
    FaIr,
    CyGb,
}

impl Locale {
    /// Map a BCP-47 language tag to the closest supported locale.
    /// Unknown primary languages fall back to [`Locale::En`].
    pub(crate) fn from_tag(tag: &LanguageTag) -> Self {
        let primary = tag.primary_language().to_ascii_lowercase();
        let region = region_subtag(tag.as_str()).map(|s| s.to_ascii_uppercase());

        match (primary.as_str(), region.as_deref()) {
            ("en", _) => Self::En,
            ("fr", _) => Self::FrFr,
            ("ja", _) => Self::JaJp,
            ("zh", Some("TW")) => Self::ZhTw,
            ("zh", _) => Self::ZhCn,
            ("de", _) => Self::DeDe,
            ("it", _) => Self::ItIt,
            ("pt", Some("BR")) => Self::PtBr,
            ("pt", _) => Self::PtPt,
            ("nl", _) => Self::NlNl,
            ("tr", _) => Self::TrTr,
            ("ar", _) => Self::ArSa,
            ("fa", _) => Self::FaIr,
            ("cy", _) => Self::CyGb,
            _ => Self::En,
        }
    }
}

/// Pull the BCP-47 region subtag out of a tag like `zh-TW` or
/// `pt-BR`. Returns `None` if the tag has no region.
fn region_subtag(tag: &str) -> Option<&str> {
    tag.split('-')
        .nth(1)
        .filter(|part| part.len() == 2 && part.chars().all(|c| c.is_ascii_alphabetic()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag(s: &str) -> LanguageTag {
        s.parse().expect("valid bcp-47")
    }

    #[test]
    fn english_variants_map_to_en() {
        assert_eq!(Locale::from_tag(&tag("en")), Locale::En);
        assert_eq!(Locale::from_tag(&tag("en-US")), Locale::En);
        assert_eq!(Locale::from_tag(&tag("en-GB")), Locale::En);
    }

    #[test]
    fn chinese_region_splits_simplified_traditional() {
        assert_eq!(Locale::from_tag(&tag("zh-CN")), Locale::ZhCn);
        assert_eq!(Locale::from_tag(&tag("zh-TW")), Locale::ZhTw);
        assert_eq!(Locale::from_tag(&tag("zh")), Locale::ZhCn);
    }

    #[test]
    fn portuguese_region_splits_brazil_portugal() {
        assert_eq!(Locale::from_tag(&tag("pt-BR")), Locale::PtBr);
        assert_eq!(Locale::from_tag(&tag("pt-PT")), Locale::PtPt);
        assert_eq!(Locale::from_tag(&tag("pt")), Locale::PtPt);
    }

    #[test]
    fn unknown_falls_back_to_english() {
        assert_eq!(Locale::from_tag(&tag("kl")), Locale::En);
    }
}
