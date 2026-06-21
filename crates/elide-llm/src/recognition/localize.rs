//! Localization: map a [`TextCandidate`] to a byte range in the
//! source text using its `context` hint.
//!
//! Both context and source are normalized (NFC + whitespace
//! collapse) before searching to absorb LLM whitespace drift. Byte
//! offsets returned are in the *original*, un-normalized text.

use unicode_normalization::UnicodeNormalization;

use crate::candidates::TextCandidate;

const TARGET: &str = "elide_llm::prompt::localize";

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
    let (normalized_text, index_maps) = normalize_with_index_map(text);

    let mut out = Vec::with_capacity(candidates.len());
    for c in candidates {
        if let Some(localized) = localize_one(&normalized_text, &index_maps, &c, policy) {
            out.push(localized);
        }
    }
    out
}

fn localize_one(
    normalized_text: &str,
    orig_index: &(Vec<usize>, Vec<usize>),
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
    let (normalized_context, _) = normalize_with_index_map(context);
    let (normalized_value, _) = normalize_with_index_map(&candidate.value);

    let context_matches: Vec<usize> = normalized_text
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
    let window = &normalized_text[context_start..context_end];
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

    let (start_index, end_index) = orig_index;
    let start_offset = *start_index.get(norm_start)?;
    let end_offset = if norm_end == 0 {
        start_offset
    } else {
        *end_index.get(norm_end - 1)?
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
        entity_id = ?c.entity_id,
        value = %c.value,
        reason,
        "dropping unresolvable text candidate"
    );
}

/// Normalize text (NFC + whitespace collapse) and return parallel
/// maps from normalized byte positions to original byte positions.
fn normalize_with_index_map(text: &str) -> (String, (Vec<usize>, Vec<usize>)) {
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
    (out, (start_index, end_index))
}
