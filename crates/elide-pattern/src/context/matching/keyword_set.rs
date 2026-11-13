//! [`KeywordSet`]: one list of context keywords, compiled for matching.

use std::collections::HashSet;
use std::iter;
use std::ops::Range;

use aho_corasick::AhoCorasick;
use elide_core::{Error, Result};
use hipstr::HipStr;

use super::by_position;

/// A list of context keywords, compiled once for case-insensitive search.
///
/// Keeps the keywords as written ([`terms`]) for a [`KeywordMatcher`] that
/// matches them its own way, and a searcher over their lowercase forms:
/// [`find_all`] reports every occurrence in a text whatever its case, Unicode
/// included (`FÜHRERSCHEIN` matches `führerschein`), and [`contains`] tests
/// whether one word is a keyword.
///
/// [`terms`]: Self::terms
/// [`find_all`]: Self::find_all
/// [`contains`]: Self::contains
/// [`KeywordMatcher`]: super::KeywordMatcher
#[derive(Debug, Clone)]
pub struct KeywordSet {
    terms: Vec<HipStr<'static>>,
    lowered: HashSet<String>,
    searcher: AhoCorasick,
}

impl KeywordSet {
    /// Compile `terms`, skipping empty ones and repeats that differ only in
    /// case.
    ///
    /// # Errors
    ///
    /// Returns a configuration error when the searcher cannot be built.
    pub fn new<T>(terms: impl IntoIterator<Item = T>) -> Result<Self>
    where
        T: Into<HipStr<'static>>,
    {
        let mut lowered = HashSet::new();
        let terms: Vec<HipStr<'static>> = terms
            .into_iter()
            .map(Into::into)
            .filter(|term: &HipStr<'static>| {
                !term.is_empty() && lowered.insert(lowercase(term.as_str()))
            })
            .collect();
        let searcher = AhoCorasick::new(terms.iter().map(|term| lowercase(term.as_str())))
            .map_err(|e| Error::configuration(format!("compiling context keywords: {e}")))?;
        Ok(Self {
            terms,
            lowered,
            searcher,
        })
    }

    /// The keywords as written.
    #[must_use]
    pub fn terms(&self) -> &[HipStr<'static>] {
        &self.terms
    }

    /// Whether the set holds no keywords.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    /// Whether `word` is one of the keywords, ignoring case.
    #[must_use]
    pub fn contains(&self, word: &str) -> bool {
        self.lowered.contains(&lowercase(word))
    }

    /// The byte range in `text` of every keyword occurrence, ignoring case,
    /// overlapping ones included. Sorted by start, the longest first among
    /// those that start together.
    #[must_use]
    pub fn find_all(&self, text: &str) -> Vec<Range<usize>> {
        let lowered = Lowered::new(text);
        let mut found: Vec<Range<usize>> = self
            .searcher
            .find_overlapping_iter(lowered.text.as_str())
            .map(|m| lowered.source_range(text, m.range()))
            .collect();
        found.sort_by(by_position);
        found
    }
}

/// `text` with every character lowercased.
///
/// Per character, so a keyword and the text it is searched in lowercase alike.
fn lowercase(text: &str) -> String {
    text.chars().flat_map(char::to_lowercase).collect()
}

/// A lowercased text and the way back to the source's byte offsets.
struct Lowered {
    text: String,
    /// For each byte of `text`, the source offset of the character it came
    /// from. `None` when the source is ASCII, which lowercases byte for byte.
    sources: Option<Vec<usize>>,
}

impl Lowered {
    fn new(source: &str) -> Self {
        if source.is_ascii() {
            return Self {
                text: source.to_ascii_lowercase(),
                sources: None,
            };
        }
        let mut text = String::with_capacity(source.len());
        let mut sources = Vec::with_capacity(source.len());
        for (offset, c) in source.char_indices() {
            for lower in c.to_lowercase() {
                text.push(lower);
                sources.extend(iter::repeat_n(offset, lower.len_utf8()));
            }
        }
        Self {
            text,
            sources: Some(sources),
        }
    }

    /// The source range a non-empty `range` of the lowercased text came from,
    /// widened to whole source characters.
    fn source_range(&self, source: &str, range: Range<usize>) -> Range<usize> {
        let Some(sources) = &self.sources else {
            return range;
        };
        let start = sources[range.start];
        let last = sources[range.end - 1];
        let end = last + source[last..].chars().next().map_or(0, char::len_utf8);
        start..end
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(terms: &[&'static str]) -> KeywordSet {
        KeywordSet::new(terms.iter().copied()).unwrap()
    }

    #[test]
    fn finds_ascii_keywords_whatever_their_case() {
        assert_eq!(set(&["ssn"]).find_all("Your SSN: 123"), vec![5..8]);
    }

    #[test]
    fn finds_unicode_keywords_whatever_their_case() {
        let text = "Nr. FÜHRERSCHEIN 123";
        let found = set(&["führerschein"]).find_all(text);
        assert_eq!(found, vec![4..17]);
        assert_eq!(&text[found[0].clone()], "FÜHRERSCHEIN");
    }

    #[test]
    fn maps_a_lengthening_lowercase_back_to_the_source() {
        // `İ` lowercases to two characters; the offsets after it still map.
        let text = "İ numer PESEL";
        let found = set(&["pesel"]).find_all(text);
        assert_eq!(&text[found[0].clone()], "PESEL");
    }

    #[test]
    fn reports_overlapping_occurrences_longest_first() {
        assert_eq!(
            set(&["karte", "kreditkarte"]).find_all("die Kreditkarte hier"),
            vec![4..15, 10..15]
        );
        assert_eq!(set(&["tax", "tax id"]).find_all("tax id"), vec![0..6, 0..3]);
    }

    #[test]
    fn skips_empty_and_repeated_keywords() {
        let keywords = set(&["", "SSN", "ssn"]);
        assert_eq!(keywords.terms(), &["SSN"]);
        assert!(keywords.contains("Ssn"));
    }
}
