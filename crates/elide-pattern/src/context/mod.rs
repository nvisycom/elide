//! Keyword-based confidence boosting for pattern and dictionary matches.
//!
//! Some matches are ambiguous on their own but become far more likely when the
//! surrounding text gives them away. A nine-digit number is just a number until
//! the word "SSN" sits beside it. A rule's `[context]` ([`RuleContext`])
//! names the keywords that raise the confidence of its matches when they appear
//! nearby, so weak-but-plausible findings clear the threshold only when their
//! context supports them.
//!
//! A rule's context applies to that rule's matches only. Each match is lifted
//! at most once by a keyword, found in the words around it or in an
//! out-of-band hint (a column header, a field name), and at most once by an
//! [`Anchor`]: a match of another label nearby. Keywords match as plain
//! substrings ([`SubstringMatcher`]), or, when a tokenizing enricher ran,
//! across word forms such as "running" and "run" ([`LemmaMatcher`]). Each lift
//! is recorded on the entity, so its effect on the final confidence is always
//! traceable.
//!
//! [`SubstringMatcher`]: matching::SubstringMatcher
//! [`LemmaMatcher`]: matching::LemmaMatcher

mod boost;
mod compiled;
mod declaration;
pub mod matching;
mod window;

pub(crate) use self::boost::{Boost, Surroundings, boost};
pub(crate) use self::compiled::CompiledContext;
pub use self::declaration::{Anchor, Matching, RuleContext};
