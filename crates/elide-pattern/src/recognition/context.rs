//! [`Context`]: per-rule keyword set used by the post-recognition
//! [`ContextEnhanced`] layer.
//!
//! Two shapes:
//!
//! - [`Global`] — one flat keyword list applied regardless of the
//!   per-call language hint.
//! - [`PerLanguage`] — keyword lists keyed by [`LanguageTag`]; the
//!   enhancer picks the entry matching `RecognizerInput.language`.
//!   When no language hint is set, the union of every per-language
//!   keyword fires (matches the crate's "missing language = any"
//!   theme used by [`Regex::languages`] / [`Dictionary::languages`]).
//!
//! [`Global`]: Context::Global
//! [`PerLanguage`]: Context::PerLanguage
//! [`ContextEnhanced`]: elide_context::ContextEnhanced
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
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, From)]
#[serde(untagged)]
pub enum Context {
    /// One flat keyword list applied regardless of the per-call
    /// language hint.
    Global(Vec<String>),
    /// Per-language keyword lists. The enhancer picks the entry
    /// matching `RecognizerInput.language`, or unions every list
    /// when no hint is set.
    PerLanguage(HashMap<LanguageTag, Vec<String>>),
}

impl Context {
    /// Return `true` when no keywords are declared in any scope.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        match self {
            Self::Global(kws) => kws.is_empty(),
            Self::PerLanguage(map) => map.values().all(Vec::is_empty),
        }
    }

    /// Iterate over `(language, keywords)` pairs.
    ///
    /// [`Global`] yields one entry with `language = None`;
    /// [`PerLanguage`] yields one entry per language.
    ///
    /// [`Global`]: Self::Global
    /// [`PerLanguage`]: Self::PerLanguage
    pub fn iter(&self) -> ContextIter<'_> {
        match self {
            Self::Global(kws) => ContextIter::Global(Some(kws.as_slice())),
            Self::PerLanguage(map) => ContextIter::PerLanguage(map.iter()),
        }
    }
}

impl Default for Context {
    fn default() -> Self {
        Self::Global(Vec::new())
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

    #[derive(Deserialize)]
    struct Wrap {
        context: Context,
    }

    #[test]
    fn parses_flat_array_as_global() {
        let toml = r#"context = ["a", "b"]"#;
        let w: Wrap = toml::from_str(toml).unwrap();
        assert_eq!(w.context, Context::Global(vec!["a".into(), "b".into()]));
    }

    #[test]
    fn parses_table_as_per_language() {
        let toml = r#"
            [context]
            en = ["card"]
            es = ["tarjeta"]
        "#;
        let w: Wrap = toml::from_str(toml).unwrap();
        let map = match w.context {
            Context::PerLanguage(m) => m,
            _ => panic!("expected PerLanguage"),
        };
        assert_eq!(map.len(), 2);
        assert_eq!(
            map.get(&LanguageTag::parse("en").unwrap()).unwrap(),
            &vec!["card".to_owned()]
        );
        assert_eq!(
            map.get(&LanguageTag::parse("es").unwrap()).unwrap(),
            &vec!["tarjeta".to_owned()]
        );
    }

    #[test]
    fn iter_global_yields_one_none_entry() {
        let ctx = Context::Global(vec!["a".into(), "b".into()]);
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
