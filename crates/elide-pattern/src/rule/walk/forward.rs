//! The walk forward from a marker.

use std::ops::Range;

use super::compiled::CompiledWalk;
use super::lowercase::{Bridge, LowercaseRun};
use super::scan::{self, Kind};

impl CompiledWalk {
    /// The name after the marker ending at `marker`, from its first word to
    /// its last, or `None` when no word follows.
    ///
    /// Unlike a backward walk, it stops at the first word that cannot be part
    /// of the name, a verb included: what follows a name is the rest of the
    /// sentence, with no lead-in to trim.
    pub(super) fn after(
        &self,
        text: &str,
        marker: usize,
        bridge: &Bridge<'_>,
    ) -> Option<Range<usize>> {
        let words = &self.lexicon;
        let mut pos = marker;
        let mut start = None;
        // The end of the last word a name can end on: not a connective
        // ("Department of the" is "Department").
        let mut end = None;
        let mut lowercase = LowercaseRun::new(words, bridge);
        // The last word stands between two names rather than inside one: `&`
        // or a party connector, not a particle ("Juan de Don Vega").
        let mut separated = false;

        for _ in 0..self.max_words {
            let Some(step) = scan::step_forward(text, pos) else {
                break;
            };
            pos = step.end;
            let word = &text[step.clone()];
            // Another marker starts another name ("Mr. and Mrs. Smith"). An
            // ambiguous one is also a name word ("Dr. Don Draper"), so it does
            // only after a separator ("Dr. John Smith & Judge Jane Doe").
            if self.markers.is_distinctive(word) || (separated && self.markers.is_marker(word)) {
                break;
            }
            // "…of Finance." ends the sentence: the word is part of the name,
            // its dot is not.
            let ends = scan::ends_sentence(word);
            let range = if ends { step.start..step.end - 1 } else { step };
            let word = &text[range.clone()];
            match scan::kind(word) {
                None | Some(Kind::Amount) => break,
                Some(Kind::Ampersand) => separated = true,
                Some(Kind::Lower) => {
                    let lower = word.to_lowercase();
                    if words.verbs.contains(&lower)
                        || !lowercase.allows(&lower)
                        || !lowercase.pass(&lower)
                    {
                        break;
                    }
                    if !words.connectors.contains(&lower) {
                        end = Some(range.end);
                    }
                    separated = words.party_connectors.contains(&lower);
                }
                Some(Kind::Capital | Kind::Number) => {
                    lowercase.reset();
                    end = Some(range.end);
                    separated = false;
                }
            }
            start.get_or_insert(range.start);
            if ends {
                break;
            }
        }
        Some(start?..end?)
    }
}
