//! Reading text word by word around an anchor.

use std::ops::Range;

/// One word before a walk position, and what separates it from that position.
pub(super) struct Step {
    /// The word's byte range.
    pub(super) range: Range<usize>,
    /// A comma sits between the word and the position.
    pub(super) comma: bool,
    /// An opening quote begins the word: it is the first word of a quoted name.
    pub(super) quoted: bool,
}

/// What a word can be in a name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    /// Starts with a capital (or a letter without case): `Acme`, `J.P.`.
    Capital,
    /// Starts with a lowercase letter: `of`, `spořitelna`.
    Lower,
    /// A number: `360`.
    Number,
    /// An amount with grouped digits: `45,000,000`, `45.000.000`.
    Amount,
    /// `&`.
    Ampersand,
}

/// The word before `pos`, skipping the spaces and at most one comma between.
///
/// `None` when a line break or punctuation other than a comma comes first, or
/// the text starts there. At the anchor (`at_anchor`), a short jurisdiction
/// marker such as `(UK)` between the name and its anchor is skipped.
pub(super) fn step_back(text: &str, pos: usize, at_anchor: bool) -> Option<Step> {
    let mut end = pos;
    let mut comma = false;
    loop {
        let (i, c) = char_before(text, end)?;
        match c {
            '\n' | '\r' => return None,
            ',' if !joins_digits(text, i) => {
                if comma {
                    return None;
                }
                comma = true;
                end = i;
            }
            ')' if at_anchor && !comma && end == pos_after_spaces(text, pos) => {
                end = jurisdiction_start(text, i)?;
            }
            c if c.is_whitespace() => end = i,
            _ => break,
        }
    }

    let mut start = end;
    while let Some((i, c)) = char_before(text, start) {
        if is_word_char(c) || (c == ',' && joins_digits(text, i)) {
            start = i;
        } else {
            break;
        }
    }
    let quoted = text[start..end].starts_with(['\'', '’']);
    if quoted {
        start += text[start..].chars().next().map_or(0, char::len_utf8);
    }
    (start < end).then_some(Step {
        range: start..end,
        comma,
        quoted,
    })
}

/// The word after `pos`, past the spaces between. `None` when a line break,
/// punctuation, or the end of the text comes first: a name after its anchor
/// never continues past a comma ("Dr. Hans Müller, CEO").
pub(super) fn step_forward(text: &str, pos: usize) -> Option<Range<usize>> {
    let rest = &text[pos..];
    let skip = rest.find(|c: char| !c.is_whitespace() || matches!(c, '\n' | '\r'))?;
    let start = pos + skip;
    let len = text[start..]
        .find(|c: char| !is_word_char(c))
        .unwrap_or(text.len() - start);
    (len > 0).then_some(start..start + len)
}

/// The words in `text[range]`, in order.
pub(super) fn words(text: &str, range: Range<usize>) -> impl Iterator<Item = Range<usize>> + '_ {
    let base = range.start;
    let slice = &text[range];
    let mut offset = 0;
    std::iter::from_fn(move || {
        let rest = &slice[offset..];
        let skip = rest.find(is_word_char)?;
        let begin = offset + skip;
        let len = slice[begin..]
            .find(|c: char| !is_word_char(c))
            .unwrap_or(slice.len() - begin);
        offset = begin + len;
        Some(base + begin..base + begin + len)
    })
}

/// What `word` can be in a name, or `None` when it cannot be part of one.
pub(super) fn kind(word: &str) -> Option<Kind> {
    let first = word.chars().next()?;
    if word == "&" {
        Some(Kind::Ampersand)
    } else if first.is_numeric() {
        let grouped = word.contains([',', '.']);
        Some(if grouped { Kind::Amount } else { Kind::Number })
    } else if first.is_uppercase() || (first.is_alphabetic() && !first.is_lowercase()) {
        Some(Kind::Capital)
    } else if first.is_lowercase() {
        Some(Kind::Lower)
    } else {
        None
    }
}

/// Whether `word` ends a sentence: a final dot after a run of two or more
/// lowercase letters (`said.`, `Inc.`) or capitals (`LTD.`). A dot after one
/// letter is an initial or an abbreviation (`J.`, `Co.`, `St.`).
pub(super) fn ends_sentence(word: &str) -> bool {
    let Some(body) = word.strip_suffix('.') else {
        return false;
    };
    let run: Vec<char> = body
        .chars()
        .rev()
        .take_while(|c| c.is_alphabetic())
        .collect();
    let lower = run.iter().filter(|c| c.is_lowercase()).count();
    let upper = run.iter().filter(|c| c.is_uppercase()).count();
    lower >= 2 || (upper >= 2 && lower == 0)
}

/// Whether an anchor may start at `at`: after the start of the text, a space,
/// or punctuation, but not inside a word or a dotted abbreviation (`S.C.` in
/// `U.S.C.`).
pub(super) fn opens(text: &str, at: usize) -> bool {
    match char_before(text, at) {
        None => true,
        Some((_, c)) if c.is_alphanumeric() => false,
        Some((i, '.')) => char_before(text, i).is_none_or(|(_, c)| !c.is_alphabetic()),
        Some(_) => true,
    }
}

/// Whether an anchor may end at `at`: before the end of the text or anything
/// but a letter or digit, so `AG` does not end inside `AGREEMENT`.
pub(super) fn closes(text: &str, at: usize) -> bool {
    text[at..]
        .chars()
        .next()
        .is_none_or(|c| !c.is_alphanumeric())
}

/// Whether a comma sits right before `at`, past any spaces.
pub(super) fn comma_before(text: &str, at: usize) -> bool {
    text[..at].trim_end().ends_with(',')
}

/// Whether the words before `at` end in a person's name with a middle
/// initial: `John A. Smith`.
pub(super) fn middle_initial_before(text: &str, at: usize) -> bool {
    let before = text[..at].trim_end();
    let word_start = before.rfind(char::is_whitespace).map_or(0, |i| i + 1);
    if !before[word_start..].starts_with(char::is_uppercase) {
        return false;
    }
    let mut rest = before[..word_start].trim_end().chars().rev();
    match (rest.next(), rest.next(), rest.next()) {
        (Some('.'), Some(initial), previous) => {
            initial.is_uppercase() && previous.is_none_or(|c| !c.is_alphabetic())
        }
        _ => false,
    }
}

/// The key an anchor is compared by: without dots or spaces, lowercased, so
/// `S.p.A.` and `SPA` compare equal.
pub(super) fn anchor_key(word: &str) -> String {
    word.chars()
        .filter(|c| !matches!(c, '.' | ' '))
        .flat_map(char::to_lowercase)
        .collect()
}

/// The key a name word is compared with word lists by: lowercased, without a
/// final dot or possessive, so `Company's` and `company` compare equal.
pub(super) fn word_key(word: &str) -> String {
    let word = word.trim_end_matches('.');
    let word = word
        .strip_suffix("'s")
        .or_else(|| word.strip_suffix("’s"))
        .unwrap_or(word);
    word.to_lowercase()
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '&' | '\'' | '’' | '-' | '.' | '/') || is_combining(c)
}

fn is_combining(c: char) -> bool {
    matches!(c, '\u{0300}'..='\u{036F}')
}

fn char_before(text: &str, at: usize) -> Option<(usize, char)> {
    text[..at].char_indices().next_back()
}

/// Whether the comma at `at` groups the digits of an amount (`45,000`).
fn joins_digits(text: &str, at: usize) -> bool {
    let before = char_before(text, at).is_some_and(|(_, c)| c.is_ascii_digit());
    let after = text[at + 1..]
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_digit());
    before && after
}

/// `pos`, moved back over the spaces before it.
fn pos_after_spaces(text: &str, pos: usize) -> usize {
    text[..pos].trim_end_matches([' ', '\t']).len()
}

/// The start of a jurisdiction marker closing at `close`: two to five
/// capitals in parentheses, `(UK)`. A one-letter `(A)` numbers a schedule.
fn jurisdiction_start(text: &str, close: usize) -> Option<usize> {
    let open = text[..close].rfind('(')?;
    let inner = &text[open + 1..close];
    let len = inner.chars().count();
    ((2..=5).contains(&len) && inner.chars().all(|c| c.is_uppercase())).then_some(open)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word_before(text: &str, pos: usize) -> Option<&str> {
        step_back(text, pos, false).map(|s| &text[s.range])
    }

    #[test]
    fn steps_over_spaces_and_one_comma() {
        assert_eq!(word_before("Bank of America, N.A.", 16), Some("America"));
        assert_eq!(word_before("Acme ,, Ltd", 8), None);
        assert_eq!(word_before("Signed\nAcme", 7), None);
        assert_eq!(word_before("“Acme", "“".len()), None);
    }

    #[test]
    fn steps_forward_to_the_next_word() {
        let text = "Dr. Hans Müller, CEO";
        let first = step_forward(text, 3).unwrap();
        assert_eq!(&text[first.clone()], "Hans");
        let second = step_forward(text, first.end).unwrap();
        assert_eq!(&text[second.clone()], "Müller");
        assert!(step_forward(text, second.end).is_none());
        assert!(step_forward("Dr.\nSmith", 3).is_none());
    }

    #[test]
    fn keeps_grouped_digits_in_one_word() {
        let text = "USD 45,000,000 Acme";
        assert_eq!(word_before(text, 15), Some("45,000,000"));
        assert_eq!(kind("45,000,000"), Some(Kind::Amount));
        assert_eq!(kind("45.000.000"), Some(Kind::Amount));
        assert_eq!(kind("360"), Some(Kind::Number));
    }

    #[test]
    fn skips_a_jurisdiction_marker_at_the_anchor() {
        let text = "Flom (UK) LLP";
        let step = step_back(text, 10, true).unwrap();
        assert_eq!(&text[step.range], "Flom");
        assert!(step_back("Schedule (A) LLC", 13, true).is_none());
    }

    #[test]
    fn an_opening_quote_begins_the_word() {
        let step = step_back("'Acme s.r.o.", 6, false).unwrap();
        assert!(step.quoted);
        assert_eq!(step.range, 1..5);
    }

    #[test]
    fn tells_sentence_ends_from_initials() {
        assert!(ends_sentence("Inc."));
        assert!(ends_sentence("said."));
        assert!(ends_sentence("LTD."));
        assert!(!ends_sentence("Co."));
        assert!(!ends_sentence("J.P."));
        assert!(!ends_sentence("St."));
        assert!(!ends_sentence("Acme"));
    }

    #[test]
    fn finds_a_middle_initial() {
        let text = "John A. Smith and";
        assert!(middle_initial_before(text, 14));
        assert!(!middle_initial_before("John Smith and", 11));
        assert!(!middle_initial_before("Acme Co. Smith and", 15));
    }

    #[test]
    fn anchor_boundaries() {
        assert!(opens("Acme AG", 5));
        assert!(!opens("PAG", 1));
        assert!(!opens("18 U.S.C.", 5));
        assert!(closes("Acme AG.", 7));
        assert!(!closes("AGREEMENT", 2));
        assert!(!closes("AGÊNCIA", 2));
    }

    #[test]
    fn words_split_on_spaces_and_commas() {
        let text = "Skadden, Arps & Flom";
        let found: Vec<&str> = words(text, 0..text.len()).map(|r| &text[r]).collect();
        assert_eq!(found, ["Skadden", "Arps", "&", "Flom"]);
    }
}
