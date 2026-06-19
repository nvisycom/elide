//! BCP 47 language tag.

use std::fmt;
use std::str::FromStr;

use hipstr::HipStr;
use oxilangtag::LanguageTag as RawLanguageTag;

/// Well-formed [BCP 47] language tag, such as `en`, `en-US`, or
/// `zh-Hant-HK`.
///
/// Wraps [`oxilangtag::LanguageTag`] over a [`HipStr`] backing store, so
/// short tags (the overwhelming common case) stay inline without a heap
/// allocation. Parsing validates the tag's structure up front; the
/// newtype therefore guarantees that any `LanguageTag` value in the
/// model is syntactically valid.
///
/// Used to record the language a recognizer is scoped to, or the
/// detected language of a span of content.
///
/// [BCP 47]: https://www.rfc-editor.org/info/bcp47
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct LanguageTag(RawLanguageTag<HipStr<'static>>);

impl LanguageTag {
    /// Parse and validate a BCP 47 language tag.
    ///
    /// Returns [`LanguageTagParseError`] if `tag` is not well-formed.
    ///
    /// [`LanguageTagParseError`]: oxilangtag::LanguageTagParseError
    pub fn parse(
        tag: impl Into<HipStr<'static>>,
    ) -> Result<Self, oxilangtag::LanguageTagParseError> {
        RawLanguageTag::parse(tag.into()).map(Self)
    }

    /// Primary language subtag (e.g. `"en"` for `"en-US"`).
    pub fn primary_language(&self) -> &str {
        self.0.primary_language()
    }

    /// Region subtag, if present (e.g. `"US"` for `"en-US"`).
    pub fn region(&self) -> Option<&str> {
        self.0.region()
    }

    /// Full tag as a string slice.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Whether this tag matches `other` at the primary-language level.
    ///
    /// Compares only the primary language subtag, so a broad tag matches its
    /// regional refinements: `"en"` matches `"en-US"` and `"en-GB"` (and
    /// vice versa), while `"en"` does not match `"fr"`. Used to decide
    /// whether a language-scoped recognizer rule applies to a hinted content
    /// language.
    pub fn matches(&self, other: &LanguageTag) -> bool {
        self.primary_language()
            .eq_ignore_ascii_case(other.primary_language())
    }
}

impl FromStr for LanguageTag {
    type Err = oxilangtag::LanguageTagParseError;

    fn from_str(tag: &str) -> Result<Self, Self::Err> {
        Self::parse(HipStr::from(tag))
    }
}

impl fmt::Display for LanguageTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0.as_str())
    }
}
