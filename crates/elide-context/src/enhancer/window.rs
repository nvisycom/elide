//! Window-slicing helpers shared by [`Enhancer::apply_rule`].
//!
//! Two coordinate systems matter here:
//!
//! - **Bytes**: source-text offsets. `word_window` walks Unicode
//!   word segments to expand an entity's `[start, end)` to
//!   `prefix`/`suffix` words on either side.
//! - **Tokens**: pre-tokenized stream from an upstream NLP engine.
//!   `slice_tokens_around` takes a `prefix`/`suffix` count and
//!   returns the contiguous token slice that covers the entity
//!   plus that many neighbours.
//!
//! Both paths feed the same downstream [`KeywordMatcher`]:
//! [`token_span`] reduces a non-empty token slice back to its
//! spanning substring for matchers that operate on raw text.
//!
//! [`Enhancer::apply_rule`]: super::Enhancer
//! [`KeywordMatcher`]: crate::matching::KeywordMatcher

use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

use crate::io::Token;

/// Walk `prefix` words before `range` and `suffix` words after, via
/// Unicode word segmentation, and return the spanning substring
/// (including any non-word whitespace and punctuation between words)
/// together with its **stream-byte start offset** in `text`. The
/// returned slice covers `range` itself plus the prefix / suffix
/// words; the entity's own bytes are always inside. The offset lets a
/// caller rebase a window-relative match into stream coordinates.
pub(super) fn word_window(
    text: &str,
    range: Range<usize>,
    prefix: usize,
    suffix: usize,
) -> (&str, usize) {
    let Range { start, end } = range;
    let prefix_text = &text[..start.min(text.len())];
    let suffix_text = &text[end.min(text.len())..];

    // `unicode_word_indices` yields `(byte_offset, word_str)` for
    // every "word" (alphanumeric run) in source order. Take the
    // last `prefix` on the prefix side, the first `suffix` on the
    // suffix side, and compute the spanning byte range.
    let prefix_words: Vec<(usize, &str)> = prefix_text.unicode_word_indices().collect();
    let prefix_take = prefix_words.len().saturating_sub(prefix);
    let prefix_byte = prefix_words
        .get(prefix_take)
        .map(|(idx, _)| *idx)
        .unwrap_or(start.min(text.len()));

    let suffix_byte = if suffix == 0 {
        end.min(text.len())
    } else {
        suffix_text
            .unicode_word_indices()
            .nth(suffix - 1)
            .map(|(idx, word)| end + idx + word.len())
            .unwrap_or(text.len())
    };

    let lo = floor_char_boundary(text, prefix_byte);
    let hi = ceil_char_boundary(text, suffix_byte.min(text.len()));
    (&text[lo..hi], lo)
}

/// Slice tokens by *count*: take `prefix` tokens before the first
/// token overlapping `range` and `suffix` tokens after the last. The
/// returned slice is contiguous.
pub(super) fn slice_tokens_around(
    tokens: &[Token],
    range: Range<usize>,
    prefix: usize,
    suffix: usize,
) -> &[Token] {
    if tokens.is_empty() {
        return &[];
    }
    // First token whose `offset.end > range.start` overlaps or follows the entity.
    let first_overlap = tokens.partition_point(|t| t.offset.end <= range.start);
    // One past the last token whose `offset.start < range.end` overlaps the entity.
    let last_overlap = tokens.partition_point(|t| t.offset.start < range.end);
    let lo = first_overlap.saturating_sub(prefix);
    let hi = (last_overlap + suffix).min(tokens.len());
    if lo >= hi {
        return &[];
    }
    &tokens[lo..hi]
}

/// Spanning substring covering `tokens` plus the entity itself, with
/// its **stream-byte start offset** in `text`. Used to give the matcher
/// a contiguous text window when slicing against the token stream; the
/// offset rebases a window-relative match into stream coordinates.
///
/// Precondition: `tokens` is non-empty. Callers must take the
/// [`word_window`] fallback path when their token slice is empty.
pub(super) fn token_span<'a>(
    text: &'a str,
    tokens: &[Token],
    range: Range<usize>,
) -> (&'a str, usize) {
    debug_assert!(!tokens.is_empty(), "token_span requires non-empty slice");
    // The window starts exactly at the first token so a lemma matcher can
    // rebase a match by `tokens[0].offset.start` and land on the same
    // window-relative coordinate the caller offsets from. The entity is
    // covered via `hi` when it extends past the last token; its leading
    // bytes (if any precede the first token) carry no keyword.
    let lo = tokens[0].offset.start;
    let hi = tokens[tokens.len() - 1].offset.end.max(range.end);
    let lo = floor_char_boundary(text, lo.min(text.len()));
    let hi = ceil_char_boundary(text, hi.min(text.len()));
    (&text[lo..hi], lo)
}

fn floor_char_boundary(s: &str, mut pos: usize) -> usize {
    while pos > 0 && !s.is_char_boundary(pos) {
        pos -= 1;
    }
    pos
}

fn ceil_char_boundary(s: &str, mut pos: usize) -> usize {
    while pos < s.len() && !s.is_char_boundary(pos) {
        pos += 1;
    }
    pos
}
