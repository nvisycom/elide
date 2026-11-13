//! Candidate localization: map a [`TextCandidate`] to a byte range in the
//! source text using its `context` hint.
//!
//! Both context and source are normalized (NFC + whitespace
//! collapse) before searching to absorb LLM whitespace drift. Byte
//! offsets returned are in the *original*, un-normalized text.

use std::ops::Range;

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::canonical_combining_class;

use crate::candidates::TextCandidate;

const TARGET: &str = "elide_llm::modality::localize";

/// Parallel maps from a byte position in normalized text back to the original,
/// un-normalized text: `start[i]` / `end[i]` are the original byte offsets that
/// bound the character at normalized byte `i`. Built alongside the normalized
/// string by [`normalize_with_index_map`].
struct OriginIndex {
    /// Original start offset for each normalized byte position.
    start: Vec<usize>,
    /// Original end offset for each normalized byte position.
    end: Vec<usize>,
}

/// Normalized text paired with the [`OriginIndex`] that maps its byte positions
/// back to the original text.
struct NormalizedText {
    text: String,
    index: OriginIndex,
}

/// A candidate resolved to a byte range in the source.
pub(super) struct LocalizedCandidate {
    pub candidate: TextCandidate,
    pub range: Range<usize>,
}

/// Localize every candidate against the source text, dropping (with a
/// warning) each whose context or value is missing or ambiguous.
pub(super) fn localize_all(text: &str, candidates: Vec<TextCandidate>) -> Vec<LocalizedCandidate> {
    let source = normalize_with_index_map(text);
    candidates
        .into_iter()
        .filter_map(|candidate| {
            let range = localize_one(&source, &candidate)?;
            Some(LocalizedCandidate { candidate, range })
        })
        .collect()
}

fn localize_one(source: &NormalizedText, candidate: &TextCandidate) -> Option<Range<usize>> {
    let Some(context) = candidate.context.as_deref() else {
        warn_dropped(candidate, "no context");
        return None;
    };
    let normalized_context = normalize(context);
    let normalized_value = normalize(&candidate.value);

    let context_matches: Vec<usize> = source
        .text
        .match_indices(&normalized_context)
        .map(|(i, _)| i)
        .collect();

    let context_start = match context_matches[..] {
        [] => {
            warn_dropped(candidate, "context not found");
            return None;
        }
        [start] => start,
        _ => {
            warn_dropped(candidate, "context ambiguous");
            return None;
        }
    };

    let context_end = context_start + normalized_context.len();
    let window = &source.text[context_start..context_end];
    let value_matches: Vec<usize> = window
        .match_indices(&normalized_value)
        .map(|(i, _)| i)
        .collect();
    let value_offset = match value_matches[..] {
        [] => {
            warn_dropped(candidate, "value not found in context");
            return None;
        }
        [offset] => offset,
        _ => {
            warn_dropped(candidate, "value ambiguous within context");
            return None;
        }
    };

    let norm_start = context_start + value_offset;
    let norm_end = norm_start + normalized_value.len();

    let index = &source.index;
    let start_offset = *index.start.get(norm_start)?;
    let end_offset = if norm_end == 0 {
        start_offset
    } else {
        *index.end.get(norm_end - 1)?
    };

    Some(start_offset..end_offset)
}

/// Warn that `candidate` was dropped for `reason`. Its value and context are
/// the detected text, so they are never logged.
fn warn_dropped(candidate: &TextCandidate, reason: &str) {
    tracing::warn!(
        target: TARGET,
        label = %candidate.label,
        reason,
        "dropping a text candidate that can't be located in the source",
    );
}

/// `text` normalized (NFC + whitespace collapse), without the map back to it.
fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    normalize_each(text, |c, _| out.push(c));
    out
}

/// Normalize text (NFC + whitespace collapse) and return it with the
/// [`OriginIndex`] mapping its byte positions back to the original text.
fn normalize_with_index_map(text: &str) -> NormalizedText {
    let mut out = String::with_capacity(text.len());
    let mut start = Vec::with_capacity(text.len() + 1);
    let mut end = Vec::with_capacity(text.len() + 1);
    normalize_each(text, |c, origin| {
        out.push(c);
        for _ in 0..c.len_utf8() {
            start.push(origin.start);
            end.push(origin.end);
        }
    });
    start.push(text.len());
    end.push(text.len());
    NormalizedText {
        text: out,
        index: OriginIndex { start, end },
    }
}

/// Feed `emit` each character of `text` normalized, with the byte range of
/// the source it came from.
///
/// A run of whitespace becomes one space. Everything else is NFC-normalized
/// a character at a time together with the combining marks that follow it,
/// the unit NFC composes (`e` + U+0301 becomes `é`), so a model echoing either
/// form matches the source.
fn normalize_each(text: &str, mut emit: impl FnMut(char, Range<usize>)) {
    let mut chars = text.char_indices().peekable();
    let mut last_was_space = false;
    while let Some((start, c)) = chars.next() {
        let mut end = start + c.len_utf8();
        if c.is_whitespace() {
            if !last_was_space {
                emit(' ', start..end);
                last_was_space = true;
            }
            continue;
        }
        last_was_space = false;
        while let Some(&(next, mark)) = chars.peek()
            && canonical_combining_class(mark) != 0
        {
            end = next + mark.len_utf8();
            chars.next();
        }
        for normalized in text[start..end].nfc() {
            emit(normalized, start..end);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(value: &str, context: &str) -> TextCandidate {
        TextCandidate {
            value: value.to_owned(),
            description: None,
            label: "person_name".to_owned(),
            confidence: None,
            context: Some(context.to_owned()),
            coreference: None,
        }
    }

    #[test]
    fn a_composed_value_matches_decomposed_source_text() {
        // The source spells é as e + U+0301; the model echoes the composed é.
        let text = "Name: Rene\u{301} Dupont, Paris";
        let localized = localize_all(text, vec![candidate("René", "Name: René Dupont")]);
        assert_eq!(localized.len(), 1);
        assert_eq!(&text[localized[0].range.clone()], "Rene\u{301}");
    }

    #[test]
    fn whitespace_runs_match_a_single_space() {
        let text = "Call  Jane\n\tDoe today";
        let localized = localize_all(text, vec![candidate("Jane Doe", "Call Jane Doe")]);
        assert_eq!(&text[localized[0].range.clone()], "Jane\n\tDoe");
    }

    #[test]
    fn an_ambiguous_context_is_dropped() {
        let localized = localize_all("Jane and Jane", vec![candidate("Jane", "Jane")]);
        assert!(localized.is_empty());
    }
}
