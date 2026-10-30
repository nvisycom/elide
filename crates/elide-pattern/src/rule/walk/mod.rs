//! [`Walk`] rule: a name found by walking from a word next to it.

mod back;
mod compiled;
mod forward;
mod lexicon;
mod lowercase;
mod scan;

use std::collections::HashMap;

use derive_builder::Builder;
use elide_core::entity::LabelRef;
use elide_core::primitive::{Confidence, CountryCode, LanguageTag};
use elide_core::{Error, ErrorKind, Result};
use serde::Deserialize;

pub(crate) use self::compiled::CompiledWalk;
use super::Term;
use crate::context::RuleContext;

/// A rule that finds a name next to one of its markers: a company name ending
/// in a legal form ("Müller Bau GmbH"), an institution around its head word
/// ("U.S. Department of Justice"), a person after a title ("Dr. Hans Müller").
///
/// The recognizer finds every marker, then walks word by word from it in the
/// walk's [`Direction`] while the words read as part of a name: capitalized
/// words, initials, numbers, and the connectives a name may contain ("of",
/// "&", "von").
///
/// - **Back** from the marker, the walk stops at a line break, a sentence end,
///   a quote, an amount, or an earlier name ("Acme GmbH and Beta GmbH" is two
///   names), then trims the per-language [`Words`] that introduce a name from
///   the front: a contract phrase ("by and between"), a verb ("is made by"),
///   or a party role ("Seller").
/// - **Forward** from the marker, the walk stops at punctuation, a sentence
///   end, or another marker ("Mr. and Mrs. Smith"), then drops trailing
///   connectives.
///
/// A name needs a capitalized word besides the marker that is not one of its
/// [`generic`](Words::generic) words: "Audit Committee" is not an institution.
///
/// # Examples
///
/// ```
/// use elide_core::entity::builtins;
/// use elide_pattern::{Direction, Term, Walk};
///
/// let firms = Walk::builder()
///     .with_name("firm")
///     .with_labels(vec![builtins::ORGANIZATION_NAME.to_ref()])
///     .with_markers(vec![Term::new("GmbH"), Term::new("LLC")])
///     .with_direction(Direction::Backward)
///     .build()
///     .expect("walk builds");
/// ```
#[derive(Debug, Clone, PartialEq, Builder, Deserialize)]
#[builder(
    name = "WalkBuilder",
    pattern = "owned",
    setter(into, strip_option, prefix = "with"),
    build_fn(error = "Error")
)]
#[serde(deny_unknown_fields)]
pub struct Walk {
    /// Human-readable identifier surfaced in trail provenance.
    pub name: String,
    /// Candidate entity labels, most-specific first. The recognizer emits the
    /// first candidate the request catalog declares.
    pub labels: Vec<LabelRef>,
    /// The words a name ends in. A term's own `score` overrides the walk's.
    #[builder(default)]
    #[serde(default)]
    pub markers: Vec<Term>,
    /// Score of a name whose marker names none.
    #[builder(default = "default_score()")]
    #[serde(default = "default_score")]
    pub score: Confidence,
    /// Which side of the marker the name lies on.
    pub direction: Direction,
    /// Whether the marker is part of the reported name: a legal form is
    /// ("Acme GmbH"), a title is not ("Dr." stays visible before "Hans
    /// Müller"). A [`Direction::Both`] walk always includes it.
    #[builder(default = "true")]
    #[serde(default = "default_include_marker")]
    pub include_marker: bool,
    /// The words that shape a name, per language. In TOML, a `[words.de]`
    /// table per language; `[words.und]` applies in every language.
    #[builder(default)]
    #[serde(default)]
    pub words: HashMap<LanguageTag, Words>,
    /// The most words a walk passes before it gives up.
    #[builder(default = "default_max_words()")]
    #[serde(default = "default_max_words")]
    pub max_words: usize,
    /// Evidence near a name that raises its confidence.
    #[builder(default)]
    #[serde(default)]
    pub context: RuleContext,
    /// BCP-47 language tags the walk applies to. Empty means "any language".
    #[builder(default)]
    #[serde(default)]
    pub languages: Vec<LanguageTag>,
    /// ISO 3166-1 alpha-2 country codes the walk applies to. Empty means "any
    /// country".
    #[builder(default)]
    #[serde(default)]
    pub countries: Vec<CountryCode>,
}

/// The words that shape a name in one language, matched case-insensitively.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Words {
    /// Which lowercase words a name may contain once it has a capitalized one.
    #[serde(default)]
    pub lowercase: Lowercase,
    /// Lowercase words a name may contain: `of`, `de`, `von`. Under
    /// [`Lowercase::Closed`], the only ones it may.
    #[serde(default)]
    pub connectors: Vec<String>,
    /// Words that join two parties rather than two words of one name: `and`,
    /// `und`. One right after a comma, or after a role or a document noun
    /// ("the Company and"), ends the name.
    #[serde(default)]
    pub party_connectors: Vec<String>,
    /// Lowercase phrases that introduce a party: `by and between`, `mit`. The
    /// name starts after the last one.
    #[serde(default)]
    pub clauses: Vec<String>,
    /// Lowercase verbs of the sentence around a name: `is`, `signed`. The name
    /// starts at the first capitalized word after the last one.
    #[serde(default)]
    pub verbs: Vec<String>,
    /// Party roles a name may be introduced by: `seller`, `the tenant`.
    #[serde(default)]
    pub roles: Vec<String>,
    /// Document nouns a name may be introduced by: `agreement`, `contract`.
    #[serde(default)]
    pub nouns: Vec<String>,
    /// Words that, before a one-letter marker, number a part of a document
    /// rather than start a name: `Schedule A LLC` is not a company.
    #[serde(default)]
    pub structural: Vec<String>,
    /// Nouns that keep a party connector inside a name: after a person's name
    /// with a middle initial, `and` joins two parties ("John A. Smith and
    /// Acme LLC") unless one of these follows it ("John A. Smith and Sons
    /// Ltd").
    #[serde(default)]
    pub firm_nouns: Vec<String>,
    /// Words that do not make a name specific on their own: "Audit
    /// Committee" and "Board of Directors" are not institutions.
    #[serde(default)]
    pub generic: Vec<String>,
    /// Words that do not make a name specific before its marker, though they
    /// do after it: "Finance Ministry" is generic, "Ministry of Finance" is
    /// not.
    #[serde(default)]
    pub generic_before: Vec<String>,
}

/// Which side of its marker a walk finds a name on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    /// Before the marker: a name ending in a legal form ("Acme GmbH").
    Backward,
    /// After the marker: a name after a title ("Dr. Hans Müller").
    Forward,
    /// Both sides: an institution around its head word ("U.S. Department of
    /// Justice").
    Both,
}

/// Which lowercase words a name may contain once it has a capitalized one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lowercase {
    /// Only the language's [`connectors`](Words::connectors). Suits languages
    /// that capitalize every word of a name, such as English.
    #[default]
    Closed,
    /// Any lowercase word, up to a few in a row. Suits languages that
    /// capitalize only a name's first word, such as Czech ("Základní škola").
    Open,
}

fn default_score() -> Confidence {
    Confidence::clamped(0.5)
}

fn default_max_words() -> usize {
    20
}

fn default_include_marker() -> bool {
    true
}

impl Walk {
    /// Start a chainable builder.
    ///
    /// Required fields: `name`, `labels`.
    #[must_use]
    pub fn builder() -> WalkBuilder {
        WalkBuilder::default()
    }

    /// Parse a walk from a TOML source.
    ///
    /// # Errors
    ///
    /// Returns a validation error when the TOML is malformed or missing
    /// required fields.
    pub fn from_toml(raw: &str) -> Result<Self> {
        toml::from_str(raw)
            .map_err(|e| Error::new(ErrorKind::Configuration, format!("walk rule TOML: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_walk_with_words_per_language() {
        let walk = Walk::from_toml(
            r#"
            name = "firm"
            labels = ["organization_name"]
            markers = [{ term = "GmbH" }, { term = "AG", score = 0.3 }]
            direction = "backward"

            [context.languages]
            de = ["gesellschaft"]

            [words.de]
            lowercase = "open"
            connectors = ["von"]
            "#,
        )
        .unwrap();
        assert_eq!(walk.markers.len(), 2);
        assert_eq!(walk.score, default_score());
        let german = &walk.words[&"de".parse::<LanguageTag>().unwrap()];
        assert_eq!(german.lowercase, Lowercase::Open);
        assert_eq!(german.connectors, ["von"]);
    }

    #[test]
    fn a_walk_in_both_directions_keeps_its_marker() {
        let walk = Walk::from_toml(
            r#"
            name = "institution"
            labels = ["organization_name"]
            direction = "both"
            include_marker = false
            "#,
        )
        .unwrap();
        assert!(CompiledWalk::compile(&walk, None).is_err());
    }
}
