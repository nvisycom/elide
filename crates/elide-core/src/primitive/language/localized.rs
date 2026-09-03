//! [`LocalizedText`]: a per-language map of values with an English-first
//! fallback.

use std::collections::HashMap;

use hipstr::HipStr;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::LanguageTag;

/// A value localized per [`LanguageTag`], with an English-first fallback.
///
/// The reusable mechanism behind any text that varies by language: a
/// [`Label`]'s display name and description, a redaction operator's bucket
/// label, and so on. English (`"en"`) is the conventional anchor,
/// constructors seed it, and [`resolve`] falls back to it (then to any
/// entry) when a requested locale is absent, so a caller that supplied
/// English always gets *some* value.
///
/// Generic over the stored value `T`, so it carries a bare `HipStr`
/// (a bucket label) or a richer struct (a label's name-plus-description)
/// equally. It is a thin wrapper over a `HashMap<LanguageTag, T>`; the
/// added value is the fallback policy in [`resolve`], kept in one place
/// rather than reimplemented at each use site.
///
/// [`Label`]: crate::entity::Label
/// [`resolve`]: LocalizedText::resolve
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(transparent))]
pub struct LocalizedText<T>(HashMap<LanguageTag, T>);

impl<T> LocalizedText<T> {
    /// A localized value anchored on its English text.
    pub fn new(english: impl Into<T>) -> Self {
        let mut map = HashMap::new();
        map.insert(LanguageTag::english(), english.into());
        Self(map)
    }

    /// Add (or replace) the value for `language`, returning `self` for
    /// chaining.
    #[must_use]
    pub fn with(mut self, language: LanguageTag, value: impl Into<T>) -> Self {
        self.0.insert(language, value.into());
        self
    }

    /// Insert the value for `language`, returning the previous one if any.
    pub fn insert(&mut self, language: LanguageTag, value: impl Into<T>) -> Option<T> {
        self.0.insert(language, value.into())
    }

    /// The value for `language`, falling back to English, then to any entry.
    ///
    /// `None` only when the map is empty (e.g. deserialized with no
    /// entries); constructors always seed English, so anything they built
    /// resolves to `Some`. Never panics.
    pub fn resolve(&self, language: &LanguageTag) -> Option<&T> {
        self.0
            .get(language)
            .or_else(|| self.0.get(&LanguageTag::english()))
            .or_else(|| self.0.values().next())
    }

    /// Whether the map has no entries at all.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl<T> FromIterator<(LanguageTag, T)> for LocalizedText<T> {
    fn from_iter<I: IntoIterator<Item = (LanguageTag, T)>>(entries: I) -> Self {
        Self(entries.into_iter().collect())
    }
}

// A bare string is English-only, so a caller can pass `"90 or older"` where
// a localized string is expected and add other languages later if needed.
// Concrete impls (not a blanket `From<Into<T>>`) so they can't collide with
// the reflexive `From<LocalizedText> for LocalizedText`.
impl From<&str> for LocalizedText<HipStr<'static>> {
    fn from(value: &str) -> Self {
        Self::new(HipStr::from(value.to_owned()))
    }
}

impl From<String> for LocalizedText<HipStr<'static>> {
    fn from(value: String) -> Self {
        Self::new(HipStr::from(value))
    }
}

impl From<HipStr<'static>> for LocalizedText<HipStr<'static>> {
    fn from(value: HipStr<'static>) -> Self {
        Self::new(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fr() -> LanguageTag {
        LanguageTag::parse("fr").unwrap()
    }

    #[test]
    fn requested_language_wins() {
        let text = LocalizedText::new("ninety or older").with(fr(), "quatre-vingt-dix ou plus");
        assert_eq!(
            text.resolve(&fr()).map(String::as_str),
            Some("quatre-vingt-dix ou plus")
        );
    }

    #[test]
    fn falls_back_to_english_when_absent() {
        let text: LocalizedText<String> = LocalizedText::new("ninety or older");
        // No French entry → English.
        assert_eq!(
            text.resolve(&fr()).map(String::as_str),
            Some("ninety or older")
        );
    }

    #[test]
    fn empty_map_resolves_to_none_without_panicking() {
        let text: LocalizedText<String> = LocalizedText::from_iter(std::iter::empty());
        assert!(text.is_empty());
        assert!(text.resolve(&LanguageTag::english()).is_none());
    }
}
