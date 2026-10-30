//! The lowercase rule: which lowercase words a name may hold on one call
//! ([`Bridge`]), and how many in a row one walk has passed ([`LowercaseRun`]).

use std::borrow::Cow;
use std::collections::HashSet;

use super::lexicon::Lexicon;

/// Lowercase words a walk passes in a row before it gives up: enough for a
/// name's inner words ("Základní škola a Mateřská škola"), not for a sentence.
const MAX_LOWERCASE_RUN: usize = 4;

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

/// The lowercase rule one walk follows: which lowercase words the call's
/// [`Bridge`] lets a name hold, and how many in a row it has passed.
pub(super) struct LowercaseRun<'a> {
    lexicon: &'a Lexicon,
    bridge: &'a Bridge<'a>,
    count: usize,
}

impl<'a> LowercaseRun<'a> {
    pub(super) fn new(lexicon: &'a Lexicon, bridge: &'a Bridge<'a>) -> Self {
        Self {
            lexicon,
            bridge,
            count: 0,
        }
    }

    /// Whether the call's languages let a name hold `lower`.
    pub(super) fn allows(&self, lower: &str) -> bool {
        self.bridge.allows(lower)
    }

    /// Pass `lower`, or `false` when it makes the run too long. A connector
    /// or party connector does not lengthen it.
    pub(super) fn pass(&mut self, lower: &str) -> bool {
        let lexicon = self.lexicon;
        if !lexicon.connectors.contains(lower) && !lexicon.party_connectors.contains(lower) {
            self.count += 1;
        }
        self.count <= MAX_LOWERCASE_RUN
    }

    /// End the run at a capitalized word or a number.
    pub(super) fn reset(&mut self) {
        self.count = 0;
    }
}
