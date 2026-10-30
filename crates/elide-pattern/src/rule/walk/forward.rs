//! The walk forward from an anchor.

use std::ops::Range;

use super::compiled::{CompiledWalk, MAX_LOWERCASE_RUN};
use super::lexicon::Bridge;
use super::scan::{self, Kind};

impl CompiledWalk {
    /// The name after the anchor ending at `anchor`, from its first word to
    /// its last, or `None` when no word follows.
    pub(super) fn after(
        &self,
        text: &str,
        anchor: usize,
        bridge: &Bridge<'_>,
    ) -> Option<Range<usize>> {
        let words = &self.lexicon;
        let mut pos = anchor;
        // Each word, and whether it is a connective a name cannot end on.
        let mut name: Vec<(Range<usize>, bool)> = Vec::new();
        let mut lowercase_run = 0;

        for _ in 0..self.max_words {
            let Some(step) = scan::step_forward(text, pos) else {
                break;
            };
            pos = step.end;
            let word = &text[step.clone()];
            // "Mr. and Mrs. Smith": another anchor starts another name.
            let linked = name.last().is_some_and(|(_, connective)| *connective);
            if self.anchors.starts_name(word, linked) {
                break;
            }
            // "…of Finance." ends the sentence: the word is part of the name,
            // its dot is not.
            let ends = scan::ends_sentence(word);
            let range = if ends { step.start..step.end - 1 } else { step };
            let word = &text[range.clone()];
            let connective = match scan::kind(word) {
                None | Some(Kind::Amount) => break,
                Some(Kind::Ampersand) => true,
                Some(Kind::Lower) => {
                    let lower = word.to_lowercase();
                    let connector = words.connectors.contains(&lower);
                    if words.verbs.contains(&lower) || (!connector && !bridge.allows(&lower)) {
                        break;
                    }
                    if !connector {
                        lowercase_run += 1;
                        if lowercase_run > MAX_LOWERCASE_RUN {
                            break;
                        }
                    }
                    connector
                }
                Some(Kind::Capital | Kind::Number) => {
                    lowercase_run = 0;
                    false
                }
            };
            name.push((range, connective));
            if ends {
                break;
            }
        }
        // "Department of the" is "Department": a name ends on a word, not a
        // connective.
        while name.last().is_some_and(|(_, connective)| *connective) {
            name.pop();
        }
        Some(name.first()?.0.start..name.last()?.0.end)
    }
}
