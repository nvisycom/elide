//! [`Chunking`]: the windowing strategy [`ChunkedRecognizer`](super::ChunkedRecognizer)
//! drives, and [`WordWindows`], its word-count implementation.
//!
//! Split out as a trait so the strategy is a seam: word windows today, a
//! token- or character-budgeted one later, without touching the rebase-and-merge
//! machinery in the recognizer. Each strategy is pure geometry over a `&str`, so
//! it is testable on plain strings with no recognizer.

use std::ops::Range;

/// A window over the source text: the byte range (a real slice of the original,
/// so offsets inside it are the full text's offsets, not a rejoined copy's) and
/// its start offset — the delta a finding inside the window is rebased by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    /// Byte offset of the window's start in the full text.
    pub start: usize,
    /// Byte offset just past the window's end in the full text.
    pub end: usize,
}

impl Window {
    /// The window's byte range in the full text.
    #[must_use]
    pub fn span(&self) -> Range<usize> {
        self.start..self.end
    }
}

/// A strategy for splitting text into overlapping windows small enough for a
/// length-limited recognizer.
///
/// Returns the windows in order; each is a byte range into the text passed in.
/// Consecutive windows overlap so an entity on a boundary still appears whole in
/// a neighbour (the amount is the strategy's own policy). Text that already fits
/// yields a single window spanning all of it, and empty text yields none.
pub trait Chunking: Send + Sync {
    /// The windows to recognize `text` over, in order.
    fn windows(&self, text: &str) -> Vec<Window>;
}

/// Window by **word count**: at most `size` whitespace-separated words per
/// window, each starting `size - overlap` words after the previous, so
/// consecutive windows share `overlap` words.
///
/// This is the GLiNER long-document strategy; [`default`](Self::default) is its
/// 384-word / 64-word-overlap setting. Word count is a proxy for a model's token
/// limit, so set `size` below the real token cap for scripts where words run
/// long.
#[derive(Debug, Clone, Copy)]
pub struct WordWindows {
    size: usize,
    overlap: usize,
}

impl WordWindows {
    /// Windows of at most `size` words overlapping by `overlap` words.
    ///
    /// # Panics
    ///
    /// Panics if `size` is zero (an empty window makes no progress) or if
    /// `overlap >= size` (a step of zero or less never advances).
    #[must_use]
    pub fn new(size: usize, overlap: usize) -> Self {
        assert!(size > 0, "window size must be non-zero");
        assert!(
            overlap < size,
            "overlap must be smaller than the window size"
        );
        Self { size, overlap }
    }
}

impl Default for WordWindows {
    /// The GLiNER2 long-document defaults: 384-word windows overlapping by 64
    /// words. The overlap is the boundary safety net — an entity split by a
    /// window edge still appears whole in the neighbouring window.
    fn default() -> Self {
        Self::new(384, 64)
    }
}

impl Chunking for WordWindows {
    fn windows(&self, text: &str) -> Vec<Window> {
        // Each word's byte range in the original text.
        let words: Vec<Range<usize>> = text
            .split_whitespace()
            .map(|word| {
                // `split_whitespace` yields slices of `text`, so the offset is exact.
                let start = word.as_ptr() as usize - text.as_ptr() as usize;
                start..start + word.len()
            })
            .collect();
        windows_over(&words, self.size, self.overlap)
    }
}

/// Window by **character count**: at most `size` Unicode scalar values per
/// window, each starting `size - overlap` characters after the previous.
///
/// A character bound is a far tighter proxy for a model's token limit than a
/// word bound — measured chars-per-token stays in a narrow band across scripts
/// (roughly 2–5 for text, dropping toward ~1 for emoji and dense punctuation),
/// where words-per-token swings by an order of magnitude and a word window can
/// overshoot a token cap many-fold. Every boundary is snapped to a UTF-8
/// character boundary, so a window never slices a multi-byte codepoint.
///
/// It is still a *proxy*, not a guarantee: set `size` conservatively below the
/// model's token cap (assume near one character per token for the worst input),
/// and prefer a tokenizer-backed strategy where an exact bound matters. For a
/// 512-token model, [`for_gliner`](Self::for_gliner) is the conservative setting
/// that holds even for emoji-dense text.
#[derive(Debug, Clone, Copy)]
pub struct CharWindows {
    size: usize,
    overlap: usize,
}

impl CharWindows {
    /// Windows of at most `size` characters overlapping by `overlap` characters.
    ///
    /// # Panics
    ///
    /// Panics if `size` is zero or if `overlap >= size` (a step of zero or less
    /// never advances).
    #[must_use]
    pub fn new(size: usize, overlap: usize) -> Self {
        assert!(size > 0, "window size must be non-zero");
        assert!(
            overlap < size,
            "overlap must be smaller than the window size"
        );
        Self { size, overlap }
    }

    /// A conservative setting for a 512-token model such as GLiNER: 500-character
    /// windows overlapping by 84 (about one sixth).
    ///
    /// The budget assumes nearly one character per token — the floor observed on
    /// emoji- and punctuation-dense text, where a larger char budget (or any word
    /// budget) blows past 512. The price is that ordinary prose, which runs
    /// several characters per token, chunks well under the cap and so issues more
    /// requests than a token-exact window would; that is the cost of a character
    /// bound that is safe across every script, pending a tokenizer-backed
    /// strategy.
    #[must_use]
    pub fn for_gliner() -> Self {
        Self::new(500, 84)
    }
}

impl Chunking for CharWindows {
    fn windows(&self, text: &str) -> Vec<Window> {
        // Each character's byte range in the original text. `char_indices` yields
        // the byte offset of every scalar, so every window boundary built from
        // these is a valid UTF-8 boundary — slicing can never split a codepoint.
        let chars: Vec<Range<usize>> = text
            .char_indices()
            .map(|(start, ch)| start..start + ch.len_utf8())
            .collect();
        windows_over(&chars, self.size, self.overlap)
    }
}

/// Window over `units` (each unit's byte range in the source, in order): at most
/// `size` units per window, stepping `size - overlap` units, each window's byte
/// range running from its first unit's start to its last unit's end.
///
/// The shared core of [`WordWindows`] and [`CharWindows`] — they differ only in
/// what a unit is (a word, a character). `size > 0` and `overlap < size` (the
/// constructors assert both), so the step is positive. A window that reaches the
/// final unit ends the sequence: a later one would be a suffix already covered.
fn windows_over(units: &[Range<usize>], size: usize, overlap: usize) -> Vec<Window> {
    let step = size - overlap;
    let mut windows = Vec::new();
    let mut first = 0;
    while first < units.len() {
        let last = (first + size).min(units.len());
        windows.push(Window {
            start: units[first].start,
            end: units[last - 1].end,
        });
        if last == units.len() {
            break;
        }
        first += step;
    }
    windows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window_texts(text: &str, size: usize, overlap: usize) -> Vec<&str> {
        WordWindows::new(size, overlap)
            .windows(text)
            .into_iter()
            .map(|w| &text[w.span()])
            .collect()
    }

    #[test]
    fn short_text_is_one_window() {
        assert_eq!(window_texts("a b c", 4, 1), vec!["a b c"]);
        assert_eq!(window_texts("a b c d", 4, 1), vec!["a b c d"]);
    }

    #[test]
    fn empty_text_yields_no_windows() {
        assert!(window_texts("", 4, 1).is_empty());
        assert!(window_texts("   ", 4, 1).is_empty());
    }

    #[test]
    fn long_text_windows_overlap_by_the_configured_words() {
        // 10 words, size 4, overlap 1 -> step 3 -> starts 0,3,6. The window at 6
        // already reaches the last word, so the start at 9 ("w9") is suppressed:
        // it would be a suffix already fully covered.
        let text = "w0 w1 w2 w3 w4 w5 w6 w7 w8 w9";
        assert_eq!(
            window_texts(text, 4, 1),
            vec!["w0 w1 w2 w3", "w3 w4 w5 w6", "w6 w7 w8 w9"]
        );
    }

    #[test]
    fn windows_carry_the_byte_offset_of_their_start() {
        let text = "alpha beta gamma delta";
        let got: Vec<usize> = WordWindows::new(2, 0)
            .windows(text)
            .into_iter()
            .map(|w| w.start)
            .collect();
        // Starts at "alpha" (0) and "gamma" (11).
        assert_eq!(got, vec![0, text.find("gamma").unwrap()]);
    }

    #[test]
    fn window_text_is_a_slice_preserving_original_offsets() {
        // Irregular whitespace: a window's text is the original slice, not a
        // single-space rejoin, so offsets inside it are the full text's offsets.
        let text = "a   b\tc";
        let w = WordWindows::new(2, 0).windows(text);
        assert_eq!(&text[w[0].span()], "a   b");
        assert_eq!(&text[w[1].span()], "c");
    }

    #[test]
    fn consecutive_windows_share_exactly_overlap_words() {
        let text = "w0 w1 w2 w3 w4 w5";
        // size 3, overlap 2 -> step 1 -> starts 0,1,2,3.
        assert_eq!(
            window_texts(text, 3, 2),
            vec!["w0 w1 w2", "w1 w2 w3", "w2 w3 w4", "w3 w4 w5"]
        );
    }

    #[test]
    fn default_is_the_gliner_long_document_setting() {
        let w = WordWindows::default();
        assert_eq!((w.size, w.overlap), (384, 64));
    }

    fn char_window_texts(text: &str, size: usize, overlap: usize) -> Vec<&str> {
        CharWindows::new(size, overlap)
            .windows(text)
            .into_iter()
            .map(|w| &text[w.span()])
            .collect()
    }

    #[test]
    fn char_windows_split_by_character_count() {
        // 6 chars, size 4, overlap 1 -> step 3 -> starts 0,3. The window at 3
        // reaches the end (chars 3..6), so no further window.
        assert_eq!(char_window_texts("abcdef", 4, 1), vec!["abcd", "def"]);
    }

    #[test]
    fn char_windows_never_split_a_multibyte_codepoint() {
        // Each CJK char is 3 bytes: byte 10 lands mid-codepoint, so a naive byte
        // window would panic. A size-2 char window must cut only on char
        // boundaries and reslice cleanly.
        let text = "日本語のテキスト"; // 8 chars, 24 bytes
        let windows = CharWindows::new(2, 0).windows(text);
        for w in &windows {
            // Slicing must not panic: every boundary is a char boundary.
            let slice = &text[w.span()];
            assert!(text.is_char_boundary(w.start) && text.is_char_boundary(w.end));
            assert_eq!(slice.chars().count(), 2, "each window holds 2 chars");
        }
        // Reassembling the non-overlapping windows reproduces the text.
        assert_eq!(
            windows.iter().map(|w| &text[w.span()]).collect::<String>(),
            text
        );
    }

    #[test]
    fn char_windows_bound_emoji_by_character_not_byte() {
        // Emoji are multi-byte (and several tokens each), the case that exposes a
        // budget tuned on Latin text. A char window counts them as one unit each,
        // so a size-3 window is exactly 3 emoji — never more, regardless of their
        // byte length.
        let text = "😀😀😀😀😀"; // 5 emoji, 4 bytes each = 20 bytes
        let windows = CharWindows::new(3, 1).windows(text);
        for w in &windows {
            let slice = &text[w.span()];
            assert!(
                slice.chars().count() <= 3,
                "window stays within the char budget"
            );
            assert!(text.is_char_boundary(w.start) && text.is_char_boundary(w.end));
        }
    }

    #[test]
    fn char_windows_short_text_is_one_window() {
        assert_eq!(char_window_texts("abc", 8, 2), vec!["abc"]);
    }

    #[test]
    fn for_gliner_is_the_conservative_512_token_setting() {
        let w = CharWindows::for_gliner();
        assert_eq!((w.size, w.overlap), (500, 84));
    }
}
