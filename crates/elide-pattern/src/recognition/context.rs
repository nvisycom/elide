//! [`Context`]: per-rule keyword set used by the post-recognition
//! [`Enhanced`] layer.
//!
//! Two shapes:
//!
//! - [`Global`] — one flat keyword list applied regardless of the
//!   per-call language hint.
//! - [`PerLanguage`] — keyword lists keyed by [`LanguageTag`]; the
//!   enhancer picks the entry matching `RecognizerContext.language`.
//!   When no language hint is set, the union of every per-language
//!   keyword fires (matches the crate's "missing language = any"
//!   theme used by [`Regex::languages`] / [`Dictionary::languages`]).
//!
//! [`Global`]: Context::Global
//! [`PerLanguage`]: Context::PerLanguage
//! [`Enhanced`]: elide_context::Enhanced
//! [`Regex::languages`]: super::Regex::languages
//! [`Dictionary::languages`]: super::Dictionary::languages

use std::collections::HashMap;
use std::collections::hash_map::Iter;

use derive_more::From;
use elide_core::primitive::LanguageTag;
use serde::Deserialize;

/// Per-rule context keyword set.
///
/// Either a single flat list ([`Global`]) or a map keyed by
/// language ([`PerLanguage`]).
///
/// [`Global`]: Self::Global
/// [`PerLanguage`]: Self::PerLanguage
#[derive(Debug, Clone, PartialEq, Deserialize, From)]
#[serde(untagged)]
pub enum Context {
    /// Keywords applied regardless of the per-call language hint: inline
    /// literals, terms drawn from named dictionaries, or both. In TOML a
    /// `[context]` table with `keywords` and/or `dictionaries`.
    Global(Sourced),
    /// Per-language keyword lists. The enhancer picks the entry
    /// matching `RecognizerContext.language`, or unions every list
    /// when no hint is set.
    PerLanguage(HashMap<LanguageTag, Vec<String>>),
}

/// A language-agnostic context keyword set: inline literals plus the terms of
/// named dictionaries.
///
/// The dictionaries' terms are resolved into the keyword set at
/// recognizer-build time — so a `monetary_amount` pattern can borrow every
/// currency name from the `currencies` dictionary without restating them
/// inline:
///
/// ```toml
/// [context]
/// keywords = ["total", "balance"]       # optional inline keywords
/// dictionaries = ["currencies"]         # plus dictionary-sourced terms
/// ```
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sourced {
    /// Inline keywords applied regardless of language, merged with the
    /// dictionary terms.
    #[serde(default)]
    pub keywords: Vec<String>,
    /// Names of dictionaries whose terms join the keyword set. The
    /// dictionaries must be registered on the same builder; an unknown
    /// name contributes nothing.
    #[serde(default)]
    pub dictionaries: Vec<String>,
    /// How a keyword matches the surrounding text. Defaults to
    /// [`Matching::Word`] (whole-word); set `match = "substring"` in TOML to
    /// let a keyword fire inside a longer word (`ssn` in `yourSSN`).
    #[serde(default, rename = "match")]
    pub matching: Matching,
    /// Additive confidence lift applied when one of these keywords fires,
    /// overriding the enhancer's default (`0.35`). Set `boost = 0.5` in TOML
    /// for a pattern whose keywords are strong evidence, or a lower value for
    /// generic ones. `None` (the default) uses the enhancer default, so an
    /// author only sets this when the default is wrong for that pattern.
    #[serde(default)]
    pub boost: Option<f32>,
}

/// How a context keyword matches the surrounding text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Matching {
    /// The keyword must match on word boundaries — `AUD` matches the token
    /// `AUD` but not the `aud` inside `audit`. The safe default.
    #[default]
    Word,
    /// The keyword may match inside a longer word — `ssn` fires in `yourSSN`.
    Substring,
}

impl Context {
    /// Return `true` when this context contributes no keywords at all —
    /// no inline keywords **and** no dictionary sources.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        match self {
            Self::Global(s) => s.keywords.is_empty() && s.dictionaries.is_empty(),
            Self::PerLanguage(map) => map.values().all(Vec::is_empty),
        }
    }

    /// Iterate over the *inline* `(language, keywords)` pairs.
    ///
    /// [`Global`] yields its inline keywords with `language = None` (its
    /// dictionary terms are resolved separately — see [`dictionaries`]);
    /// [`PerLanguage`] yields one entry per language.
    ///
    /// [`Global`]: Self::Global
    /// [`PerLanguage`]: Self::PerLanguage
    /// [`dictionaries`]: Self::dictionaries
    pub fn iter(&self) -> ContextIter<'_> {
        match self {
            Self::Global(s) => ContextIter::Global(Some(s.keywords.as_slice())),
            Self::PerLanguage(map) => ContextIter::PerLanguage(map.iter()),
        }
    }

    /// The names of the dictionaries this context sources keywords from, if
    /// any. Non-empty only for a [`Global`] context declaring `dictionaries`;
    /// the recognizer resolves each name against its registered dictionaries
    /// and merges their terms into the boost keyword set at build time.
    ///
    /// [`Global`]: Self::Global
    #[must_use]
    pub fn dictionaries(&self) -> &[String] {
        match self {
            Self::Global(s) => &s.dictionaries,
            Self::PerLanguage(_) => &[],
        }
    }

    /// How this context's keywords match the surrounding text. A [`Global`]
    /// context carries its declared mode; a [`PerLanguage`] context always
    /// matches on word boundaries.
    ///
    /// [`Global`]: Self::Global
    /// [`PerLanguage`]: Self::PerLanguage
    #[must_use]
    pub fn matching(&self) -> Matching {
        match self {
            Self::Global(s) => s.matching,
            Self::PerLanguage(_) => Matching::Word,
        }
    }

    /// The additive boost override this context declares, if any. `Some` only
    /// for a [`Global`] context with an explicit `boost`; `None` (a
    /// [`PerLanguage`] context, or a `Global` one without the field) leaves
    /// the enhancer's default boost in place.
    ///
    /// [`Global`]: Self::Global
    /// [`PerLanguage`]: Self::PerLanguage
    #[must_use]
    pub fn boost(&self) -> Option<f32> {
        match self {
            Self::Global(s) => s.boost,
            Self::PerLanguage(_) => None,
        }
    }
}

impl Default for Context {
    fn default() -> Self {
        Self::Global(Sourced::default())
    }
}

impl From<Vec<String>> for Context {
    /// A bare keyword list becomes a language-agnostic [`Global`] context with
    /// no dictionary sources — the ergonomic path for building a context from
    /// keywords in code.
    ///
    /// [`Global`]: Self::Global
    fn from(keywords: Vec<String>) -> Self {
        Self::Global(Sourced {
            keywords,
            ..Sourced::default()
        })
    }
}

/// Iterator returned by [`Context::iter`].
pub enum ContextIter<'a> {
    Global(Option<&'a [String]>),
    PerLanguage(Iter<'a, LanguageTag, Vec<String>>),
}

impl<'a> Iterator for ContextIter<'a> {
    type Item = (Option<&'a LanguageTag>, &'a [String]);

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Global(slot) => slot.take().map(|kws| (None, kws)),
            Self::PerLanguage(it) => it.next().map(|(lang, kws)| (Some(lang), kws.as_slice())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn global(keywords: &[&str], dictionaries: &[&str]) -> Context {
        Context::Global(Sourced {
            keywords: keywords.iter().map(|s| (*s).to_owned()).collect(),
            dictionaries: dictionaries.iter().map(|s| (*s).to_owned()).collect(),
            ..Sourced::default()
        })
    }

    #[test]
    fn dictionaries_accessor_returns_the_sources() {
        // A Global context surfaces its dictionary names; a PerLanguage context
        // sources no dictionaries.
        assert_eq!(
            global(&["total"], &["currencies", "cryptocurrencies"]).dictionaries(),
            ["currencies", "cryptocurrencies"]
        );
        let mut map = HashMap::new();
        map.insert(LanguageTag::parse("en").unwrap(), vec!["card".to_owned()]);
        assert!(Context::PerLanguage(map).dictionaries().is_empty());
    }

    #[test]
    fn is_empty_only_when_no_keywords_and_no_dictionaries() {
        assert!(global(&[], &[]).is_empty());
        assert!(!global(&["a"], &[]).is_empty());
        assert!(!global(&[], &["currencies"]).is_empty());
    }

    #[test]
    fn iter_global_yields_one_none_entry() {
        let ctx = global(&["a", "b"], &[]);
        let collected: Vec<_> = ctx
            .iter()
            .map(|(lang, kws)| (lang.cloned(), kws.to_vec()))
            .collect();
        assert_eq!(collected.len(), 1);
        assert!(collected[0].0.is_none());
        assert_eq!(collected[0].1, vec!["a".to_owned(), "b".to_owned()]);
    }

    #[test]
    fn iter_per_language_yields_one_entry_per_language() {
        let mut map = HashMap::new();
        map.insert(LanguageTag::parse("en").unwrap(), vec!["card".into()]);
        map.insert(LanguageTag::parse("es").unwrap(), vec!["tarjeta".into()]);
        let ctx = Context::PerLanguage(map);
        let collected: Vec<_> = ctx
            .iter()
            .map(|(lang, kws)| (lang.unwrap().to_string(), kws.to_vec()))
            .collect();
        assert_eq!(collected.len(), 2);
    }

    #[test]
    fn default_is_empty_global() {
        let ctx = Context::default();
        assert!(ctx.is_empty());
    }
}
