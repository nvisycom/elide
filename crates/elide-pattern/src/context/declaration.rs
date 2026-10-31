//! [`RuleContext`]: the `[context]` table a rule declares.

use std::collections::HashMap;

use elide_core::primitive::LanguageTag;
use serde::Deserialize;

/// The context a [`Pattern`] or [`Dictionary`] declares: the evidence near a
/// match that raises its confidence.
///
/// ```toml
/// [context]
/// keywords = ["total", "balance"]       # any language
/// dictionaries = ["currencies"]         # plus a dictionary's terms
/// boost = 0.5                           # lift per signal (default 0.35)
/// anchor = { label = "currency" }       # a nearby match of this label
///
/// [context.languages]                   # keywords for one language each
/// en = ["card", "credit"]
/// de = ["karte", "kreditkarte"]
/// ```
///
/// A per-language list applies when the call asserts its language; with none
/// asserted, every list applies (matches the crate's "missing language = any"
/// theme used by [`Pattern::languages`] / [`Dictionary::languages`]).
///
/// [`Pattern`]: crate::Pattern
/// [`Dictionary`]: crate::Dictionary
/// [`Pattern::languages`]: crate::Pattern::languages
/// [`Dictionary::languages`]: crate::Dictionary::languages
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleContext {
    /// Keywords that apply in any language.
    #[serde(default)]
    pub keywords: Vec<String>,
    /// Keywords that apply only in their language.
    #[serde(default)]
    pub languages: HashMap<LanguageTag, Vec<String>>,
    /// Names of dictionaries whose terms join [`keywords`](Self::keywords).
    /// The dictionaries must be registered on the same builder; an unknown
    /// name contributes nothing.
    #[serde(default)]
    pub dictionaries: Vec<String>,
    /// How a keyword matches the surrounding text. Defaults to
    /// [`Matching::Word`] (whole-word); set `match = "substring"` in TOML to
    /// let a keyword fire inside a longer word (`ssn` in `yourSSN`).
    #[serde(default, rename = "match")]
    pub matching: Matching,
    /// Confidence lift applied when a keyword or the anchor fires. `None` (the
    /// default) lifts by `0.35`; set it higher for a rule whose keywords are
    /// strong evidence, or lower for generic ones.
    #[serde(default)]
    pub boost: Option<f32>,
    /// Lift a match when a match of the named label sits nearby: a stronger
    /// signal than a keyword for a value whose meaning depends on an adjacent
    /// entity (a number that is a monetary amount only beside a currency).
    #[serde(default)]
    pub anchor: Option<Anchor>,
}

impl RuleContext {
    /// Return `true` when this context contributes no keywords: no inline
    /// keywords in any language and no dictionary sources.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.keywords.is_empty()
            && self.dictionaries.is_empty()
            && self.languages.values().all(Vec::is_empty)
    }
}

impl From<Vec<String>> for RuleContext {
    /// Keywords that apply in any language, the ergonomic path for building a
    /// context in code.
    fn from(keywords: Vec<String>) -> Self {
        Self {
            keywords,
            ..Self::default()
        }
    }
}

/// A context [`anchor`](RuleContext::anchor): the neighbour match's `label`
/// and the least confidence it must carry. `min_confidence` defaults to the
/// acceptance threshold, so a bare `{ label = "currency" }` requires a
/// confirmed neighbour.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Anchor {
    /// The neighbour match's label.
    pub label: String,
    /// The confidence floor the neighbour must clear; `None` means the
    /// acceptance threshold.
    #[serde(default)]
    pub min_confidence: Option<f32>,
}

/// How a context keyword matches the surrounding text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Matching {
    /// The keyword must match on word boundaries, `AUD` matches the token
    /// `AUD` but not the `aud` inside `audit`. The safe default.
    #[default]
    Word,
    /// The keyword may match inside a longer word, `ssn` fires in `yourSSN`.
    Substring,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_field() {
        let context: RuleContext = toml::from_str(
            r#"
            keywords = ["total"]
            dictionaries = ["currencies"]
            match = "substring"
            boost = 0.5
            anchor = { label = "currency", min_confidence = 0.1 }

            [languages]
            en = ["card"]
            "#,
        )
        .unwrap();
        assert_eq!(context.keywords, ["total"]);
        assert_eq!(context.dictionaries, ["currencies"]);
        assert_eq!(context.matching, Matching::Substring);
        assert_eq!(context.boost, Some(0.5));
        assert_eq!(context.anchor.unwrap().label, "currency");
        assert_eq!(context.languages.len(), 1);
    }

    #[test]
    fn is_empty_only_without_keywords_in_any_language_or_dictionaries() {
        assert!(RuleContext::default().is_empty());
        assert!(!RuleContext::from(vec!["a".to_owned()]).is_empty());
        let sourced = RuleContext {
            dictionaries: vec!["currencies".to_owned()],
            ..RuleContext::default()
        };
        assert!(!sourced.is_empty());
        let mut languages = HashMap::new();
        languages.insert("en".parse().unwrap(), vec!["card".to_owned()]);
        let per_language = RuleContext {
            languages,
            ..RuleContext::default()
        };
        assert!(!per_language.is_empty());
    }
}
