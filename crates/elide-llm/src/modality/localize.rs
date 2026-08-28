//! Candidate localization: map a [`TextCandidate`] to a byte range in the
//! source text using its `context` hint.
//!
//! Both context and source are normalized (NFC + whitespace
//! collapse) before searching to absorb LLM whitespace drift. Byte
//! offsets returned are in the *original*, un-normalized text.

use unicode_normalization::UnicodeNormalization;

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

/// A candidate that's been resolved to a byte range in the source.
#[derive(Debug, Clone)]
pub(super) struct LocalizedCandidate {
    pub candidate: TextCandidate,
    pub start_offset: usize,
    pub end_offset: usize,
}

/// What to do with candidates that can't be uniquely localized.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[derive(serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub(super) enum UnresolvedCandidatePolicy {
    /// Drop ambiguous and missing candidates. Default.
    #[default]
    Drop,
    /// Pick the first match for ambiguous candidates; drop only
    /// when there are zero matches.
    FirstMatch,
}

/// Localize every candidate against the source text.
pub(super) fn localize_all(
    text: &str,
    candidates: Vec<TextCandidate>,
    policy: UnresolvedCandidatePolicy,
) -> Vec<LocalizedCandidate> {
    let normalized = normalize_with_index_map(text);

    let mut out = Vec::with_capacity(candidates.len());
    for c in candidates {
        if let Some(localized) = localize_one(&normalized, &c, policy) {
            out.push(localized);
        }
    }
    out
}

fn localize_one(
    source: &NormalizedText,
    candidate: &TextCandidate,
    policy: UnresolvedCandidatePolicy,
) -> Option<LocalizedCandidate> {
    let context = match candidate.context.as_deref() {
        Some(c) => c,
        None => {
            warn_dropped(candidate, "no context");
            return None;
        }
    };
    let normalized_context = normalize_with_index_map(context).text;
    let normalized_value = normalize_with_index_map(&candidate.value).text;

    let context_matches: Vec<usize> = source
        .text
        .match_indices(&normalized_context)
        .map(|(i, _)| i)
        .collect();

    let context_start = match context_matches.len() {
        0 => {
            warn_dropped(candidate, "context not found");
            return None;
        }
        1 => context_matches[0],
        _ => match policy {
            UnresolvedCandidatePolicy::FirstMatch => context_matches[0],
            _ => {
                warn_dropped(candidate, "context ambiguous");
                return None;
            }
        },
    };

    let context_end = context_start + normalized_context.len();
    let window = &source.text[context_start..context_end];
    let value_matches: Vec<usize> = window
        .match_indices(&normalized_value)
        .map(|(i, _)| i)
        .collect();
    let value_offset = match value_matches.len() {
        0 => {
            warn_dropped(candidate, "value not found in context");
            return None;
        }
        1 => value_matches[0],
        _ => match policy {
            UnresolvedCandidatePolicy::FirstMatch => value_matches[0],
            _ => {
                warn_dropped(candidate, "value ambiguous within context");
                return None;
            }
        },
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

    Some(LocalizedCandidate {
        candidate: candidate.clone(),
        start_offset,
        end_offset,
    })
}

fn warn_dropped(c: &TextCandidate, reason: &str) {
    tracing::warn!(
        target: TARGET,
        coreference = ?c.coreference,
        value = %c.value,
        reason,
        "dropping unresolvable text candidate"
    );
}

/// Normalize text (NFC + whitespace collapse) and return it with the
/// [`OriginIndex`] mapping its byte positions back to the original text.
fn normalize_with_index_map(text: &str) -> NormalizedText {
    let mut out = String::with_capacity(text.len());
    let mut start_index: Vec<usize> = Vec::with_capacity(text.len());
    let mut end_index: Vec<usize> = Vec::with_capacity(text.len());
    let mut last_was_space = false;
    let mut orig_pos = 0usize;

    for orig_ch in text.chars() {
        let orig_ch_len = orig_ch.len_utf8();
        let orig_end = orig_pos + orig_ch_len;

        if orig_ch.is_whitespace() {
            if !last_was_space {
                start_index.push(orig_pos);
                end_index.push(orig_end);
                out.push(' ');
                last_was_space = true;
            }
        } else {
            for nfc_ch in orig_ch.to_string().nfc() {
                for _ in 0..nfc_ch.len_utf8() {
                    start_index.push(orig_pos);
                    end_index.push(orig_end);
                }
                out.push(nfc_ch);
            }
            last_was_space = false;
        }
        orig_pos += orig_ch_len;
    }
    start_index.push(orig_pos);
    end_index.push(orig_pos);
    NormalizedText {
        text: out,
        index: OriginIndex {
            start: start_index,
            end: end_index,
        },
    }
}
