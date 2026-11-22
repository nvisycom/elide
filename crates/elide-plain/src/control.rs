//! Terminal control sequences in plain text: what a terminal obeys rather
//! than shows.
//!
//! A terminal printing text acts on its control characters and escape
//! sequences: it moves the cursor, erases, recolours, conceals, retitles the
//! window. What a person sees is then not the text detection reads: a
//! sequence inside a name (`Ja\e[mne`) shows as `Jane` but splits it for
//! detection, and a cursor move or a concealing style hides what was printed.
//! [`strip`] removes every such sequence and character, so what ships is the
//! text as both read it.
//!
//! Removed, each counted once:
//!
//! - an escape sequence, introduced by `ESC` (U+001B): a control sequence
//!   (`ESC [`, CSI) through its final byte; a command string (`ESC ]` OSC,
//!   `ESC P` DCS, `ESC X` SOS, `ESC ^` PM, `ESC _` APC) through its
//!   terminator, BEL or `ESC \`, or, unterminated, to the end of its line; any
//!   other escape through its final byte. As a terminal does, a sequence reads
//!   on past a control inside it, which takes effect (a line break stays), ends
//!   at CAN or SUB, which cancel it, and ends before an `ESC`, which starts
//!   the next;
//! - every other C0 control but tab, line feed, form feed, and carriage
//!   return, which a terminal shows as space and line breaks; DEL; and each C1
//!   control (U+0080–U+009F) alone.
//!
//! A terminal reading UTF-8 does not start a sequence at a C1 control, and one
//! is what Windows-1252 text read as Latin-1 leaves for `ž` or `Ÿ`, so it never
//! takes the text after it. Nor does an unterminated command string take more
//! than its line: a terminal would swallow on, but text detection reads and no
//! one sees costs at most a redaction, where a stray `ESC ]` taking the rest of
//! a file loses it.

use std::borrow::Cow;
use std::iter::Peekable;
use std::ops::RangeInclusive;
use std::str::Chars;

use elide_codec::StrippedContent;
use elide_core::redaction::HiddenContent;

/// `ESC`, which introduces an escape sequence.
const ESC: char = '\u{1b}';
/// `BEL`, which ends a command string.
const BEL: char = '\u{07}';
/// `CAN` and `SUB`, which cancel a sequence.
const CAN: char = '\u{18}';
const SUB: char = '\u{1a}';

/// `text` without its terminal control sequences and control characters, and
/// how many were removed; borrowed when there are none.
pub(crate) fn strip(text: &str) -> (Cow<'_, str>, usize) {
    if !text.chars().any(is_control) {
        return (Cow::Borrowed(text), 0);
    }
    let mut out = String::with_capacity(text.len());
    let mut removed = 0;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if !is_control(c) {
            out.push(c);
            continue;
        }
        removed += 1;
        if c != ESC {
            continue;
        }
        match introduced(&mut chars, &mut out) {
            // Parameters (`0`–`?`) and intermediates (space–`/`), then the
            // final byte (`@`–`~`).
            Some(Introducer::Control) => skip_sequence(&mut chars, &mut out, ' '..='?'),
            Some(Introducer::String) => skip_string(&mut chars),
            // Intermediates (space–`/`), then the final byte (`0`–`~`).
            Some(Introducer::Escape) => skip_sequence(&mut chars, &mut out, ' '..='/'),
            None => {}
        }
    }
    (Cow::Owned(out), removed)
}

/// [`strip`] over an owned `text`, handed back unchanged when it holds no
/// control.
pub(crate) fn strip_owned(text: String) -> (String, usize) {
    match strip(&text) {
        (Cow::Owned(stripped), removed) => (stripped, removed),
        (Cow::Borrowed(_), _) => (text, 0),
    }
}

/// The record of `removed` control sequences and characters stripped, as
/// [`Undisplayed`](HiddenContent::Undisplayed) content.
pub(crate) fn recorded(removed: usize) -> StrippedContent {
    let mut stripped = StrippedContent::new();
    stripped.record(HiddenContent::Undisplayed, None, removed);
    stripped
}

/// Whether `c` is a control a terminal obeys: a C0 control but tab, line
/// feed, form feed, and carriage return, DEL, or a C1 control.
fn is_control(c: char) -> bool {
    c.is_control() && !matches!(c, '\t' | '\n' | '\x0c' | '\r')
}

/// What follows an escape's introducer.
enum Introducer {
    /// A control sequence (CSI): parameters, intermediates, a final byte.
    Control,
    /// A command string (OSC, DCS, SOS, PM, APC), up to its terminator.
    String,
    /// Any other escape: intermediates, a final byte.
    Escape,
}

/// The sequence an `ESC` introduces, consuming its second character when that
/// names the kind, or `None` for a lone `ESC`.
fn introduced(chars: &mut Peekable<Chars<'_>>, out: &mut String) -> Option<Introducer> {
    match peek_through(chars, out)? {
        '[' => chars.next().map(|_| Introducer::Control),
        ']' | 'P' | 'X' | '^' | '_' => chars.next().map(|_| Introducer::String),
        // Its next character is an intermediate or the final one.
        ' '..='~' => Some(Introducer::Escape),
        _ => None,
    }
}

/// Skip a sequence's characters in `body` through the printable ASCII one
/// after them, its final byte, stopping before anything else.
fn skip_sequence(chars: &mut Peekable<Chars<'_>>, out: &mut String, body: RangeInclusive<char>) {
    while let Some(c) = peek_through(chars, out) {
        chars.next_if(|_| matches!(c, ' '..='~'));
        if !body.contains(&c) {
            return;
        }
    }
}

/// The next character of an escape or control sequence, unconsumed, past the
/// controls a terminal acts on inside one: tab, line feed, form feed, and
/// carriage return go to `out`, every other C0 control and DEL go. `None` at
/// the end, at CAN or SUB, consumed as they cancel the sequence, and at `ESC`,
/// left to start the next.
fn peek_through(chars: &mut Peekable<Chars<'_>>, out: &mut String) -> Option<char> {
    while let Some(&c) = chars.peek() {
        match c {
            ESC => return None,
            CAN | SUB => {
                chars.next();
                return None;
            }
            '\t' | '\n' | '\x0c' | '\r' => out.push(c),
            '\0'..='\x1f' | '\x7f' => {}
            _ => return Some(c),
        }
        chars.next();
    }
    None
}

/// Skip a command string through its terminator: BEL, `ESC \`, or CAN or SUB,
/// which cancel it. Any other `ESC` ends it unconsumed, starting the next
/// sequence, and, unterminated, it ends with its line, keeping the line break.
fn skip_string(chars: &mut Peekable<Chars<'_>>) {
    while let Some(&c) = chars.peek() {
        match c {
            '\n' | '\r' => return,
            ESC => {
                let mut ahead = chars.clone();
                ahead.next();
                if ahead.next() == Some('\\') {
                    *chars = ahead;
                }
                return;
            }
            BEL | CAN | SUB => {
                chars.next();
                return;
            }
            _ => {
                chars.next();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stripped(text: &str) -> (String, usize) {
        let (out, removed) = strip(text);
        (out.into_owned(), removed)
    }

    #[test]
    fn plain_text_is_borrowed() {
        let (out, removed) = strip("Jane Roe\tjane@example.com\r\n");
        assert!(matches!(out, Cow::Borrowed(_)));
        assert_eq!(removed, 0);
    }

    #[test]
    fn a_sequence_inside_a_name_goes() {
        assert_eq!(stripped("Ja\u{1b}[mne Roe"), ("Jane Roe".into(), 1));
        assert_eq!(
            stripped("\u{1b}[1;31mJane\u{1b}[0m Roe"),
            ("Jane Roe".into(), 2)
        );
    }

    #[test]
    fn cursor_moves_and_concealment_go() {
        assert_eq!(stripped("Jane\u{1b}[4Dxxxx"), ("Janexxxx".into(), 1));
        assert_eq!(stripped("\u{1b}[8mJane Roe"), ("Jane Roe".into(), 1));
        assert_eq!(stripped("\u{1b}[?25l"), (String::new(), 1));
    }

    /// A control inside a sequence takes effect and the sequence reads on;
    /// CAN cancels it, and an `ESC` starts the next.
    #[test]
    fn controls_inside_a_sequence_act() {
        assert_eq!(stripped("Ja\u{1b}[\0mne"), ("Jane".into(), 1));
        assert_eq!(stripped("Ja\u{1b}[1\n;2mne"), ("Ja\nne".into(), 1));
        assert_eq!(stripped("Ja\u{1b}(\u{7}Bne"), ("Jane".into(), 1));
        assert_eq!(stripped("Ja\u{1b}[1\u{18}mne"), ("Jamne".into(), 1));
        assert_eq!(stripped("Ja\u{1b}[1\u{1b}[mne"), ("Jane".into(), 2));
    }

    #[test]
    fn command_strings_go_through_their_terminator() {
        // A window title, ended by BEL; a hyperlink, ended by ST.
        assert_eq!(stripped("\u{1b}]0;title\u{7}Jane"), ("Jane".into(), 1));
        assert_eq!(
            stripped("\u{1b}]8;;https://example.com\u{1b}\\Jane\u{1b}]8;;\u{1b}\\"),
            ("Jane".into(), 2)
        );
        // CAN cancels one, and another escape ends it and goes too.
        assert_eq!(
            stripped("before\u{1b}]0;title\u{18}after\n"),
            ("beforeafter\n".into(), 1)
        );
        assert_eq!(stripped("\u{1b}]0;title\u{1b}[mJane"), ("Jane".into(), 2));
        // Unterminated, it ends with its line.
        assert_eq!(stripped("Jane\u{1b}]0;Roe"), ("Jane".into(), 1));
        assert_eq!(
            stripped("Jane\u{1b}]0;x\r\nRoe\nDoe"),
            ("Jane\r\nRoe\nDoe".into(), 1)
        );
    }

    #[test]
    fn other_escapes_and_controls_go() {
        // A character-set designation, a reset, a backspace, a bell, DEL.
        assert_eq!(stripped("\u{1b}(BJane"), ("Jane".into(), 1));
        assert_eq!(stripped("\u{1b}cJane"), ("Jane".into(), 1));
        assert_eq!(stripped("Janx\u{8}e\u{7}\u{7f}"), ("Janxe".into(), 3));
        // A lone ESC at the end, and one before a control.
        assert_eq!(stripped("Jane\u{1b}"), ("Jane".into(), 1));
        assert_eq!(stripped("Jane\u{1b}\u{1b}[m"), ("Jane".into(), 2));
    }

    /// A C1 control goes alone: a terminal reading UTF-8 starts no sequence
    /// at one, and Windows-1252 text misread leaves them for letters.
    #[test]
    fn c1_controls_go_alone() {
        assert_eq!(stripped("Ja\u{85}ne"), ("Jane".into(), 1));
        assert_eq!(stripped("\u{9b}31mJane"), ("31mJane".into(), 1));
        assert_eq!(
            stripped("Ja\u{9e}ne Roe\nJohn Doe"),
            ("Jane Roe\nJohn Doe".into(), 1)
        );
    }

    #[test]
    fn tab_line_endings_and_form_feed_stay() {
        assert_eq!(
            stripped("a\tb\r\nc\n\u{c}d"),
            ("a\tb\r\nc\n\u{c}d".into(), 0)
        );
    }
}
