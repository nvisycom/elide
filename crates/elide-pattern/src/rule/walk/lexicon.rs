//! [`Lexicon`]: a walk's words, merged across languages.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

use elide_core::primitive::LanguageTag;

use super::{Lowercase, Words};

/// The tag of language-neutral words, which apply in every language.
const ANY_LANGUAGE: &str = "und";

/// A walk's words, lowercased and merged across its languages. Only the
/// lowercase rule depends on the call's languages; every other list applies
/// whatever the language, since a name's surroundings carry no language tag.
pub(super) struct Lexicon {
    /// Each language's lowercase rule and connectors.
    bridges: Vec<(LanguageTag, Lowercase, HashSet<String>)>,
    pub(super) connectors: HashSet<String>,
    pub(super) party_connectors: HashSet<String>,
    /// Clause phrases, longest first.
    pub(super) clauses: Vec<String>,
    pub(super) verbs: HashSet<String>,
    /// Role phrases, as word sequences.
    pub(super) roles: Vec<Vec<String>>,
    /// Single-word roles and document nouns: the words that introduce a name.
    introducers: HashSet<String>,
    pub(super) structural: HashSet<String>,
    pub(super) firm_nouns: HashSet<String>,
    pub(super) generic: HashSet<String>,
    pub(super) generic_before: HashSet<String>,
}

/// Which lowercase words a name may contain on one call.
pub(super) enum Bridge<'a> {
    /// Any lowercase word.
    Open,
    /// Only these.
    Closed(Cow<'a, HashSet<String>>),
}

impl Bridge<'_> {
    pub(super) fn allows(&self, word: &str) -> bool {
        match self {
            Self::Open => true,
            Self::Closed(words) => words.contains(word),
        }
    }
}

impl Lexicon {
    pub(super) fn new(words: &HashMap<LanguageTag, Words>) -> Self {
        let lower = |list: &[String]| -> HashSet<String> {
            list.iter().map(|w| w.to_lowercase()).collect()
        };
        let all = |pick: fn(&Words) -> &Vec<String>| -> HashSet<String> {
            words.values().flat_map(|w| lower(pick(w))).collect()
        };

        let mut clauses: Vec<String> = all(|w| &w.clauses).into_iter().collect();
        clauses.sort_by_key(|c| std::cmp::Reverse(c.len()));
        let role_phrases = all(|w| &w.roles);
        let roles = role_phrases
            .iter()
            .map(|r| r.split_whitespace().map(str::to_owned).collect())
            .collect();
        let mut introducers: HashSet<String> = role_phrases
            .into_iter()
            .filter(|r| !r.contains(' '))
            .collect();
        introducers.extend(all(|w| &w.nouns));

        Self {
            bridges: words
                .iter()
                .map(|(language, w)| (language.clone(), w.lowercase, lower(&w.connectors)))
                .collect(),
            connectors: all(|w| &w.connectors),
            party_connectors: all(|w| &w.party_connectors),
            clauses,
            verbs: all(|w| &w.verbs),
            roles,
            introducers,
            structural: all(|w| &w.structural),
            firm_nouns: all(|w| &w.firm_nouns),
            generic: all(|w| &w.generic),
            generic_before: all(|w| &w.generic_before),
        }
    }

    /// The lowercase words a name may contain under `languages`.
    ///
    /// Language-neutral words always apply; a language's words apply when
    /// `languages` names it. Any lowercase word is allowed when every
    /// language-specific entry that applies is open; otherwise only their
    /// connectors are. When `languages` names none of the walk's languages,
    /// the connectors of all of them apply.
    pub(super) fn bridge(&self, languages: &[&LanguageTag]) -> Bridge<'_> {
        let active: Vec<_> = self
            .bridges
            .iter()
            .filter(|(tag, ..)| {
                tag.as_str() == ANY_LANGUAGE || languages.iter().any(|l| tag.matches(l))
            })
            .collect();
        if active.iter().all(|(tag, ..)| tag.as_str() == ANY_LANGUAGE) {
            return Bridge::Closed(Cow::Borrowed(&self.connectors));
        }
        if active
            .iter()
            .all(|(_, lowercase, _)| *lowercase == Lowercase::Open)
        {
            return Bridge::Open;
        }
        Bridge::Closed(Cow::Owned(
            active
                .iter()
                .flat_map(|(.., words)| words.iter().cloned())
                .collect(),
        ))
    }

    /// Whether `word` is a party role or a document noun.
    pub(super) fn introduces(&self, word: &str) -> bool {
        self.introducers.contains(&word.to_lowercase())
    }
}
