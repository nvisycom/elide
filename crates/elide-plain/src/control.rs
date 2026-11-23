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
//! A terminal's own parser decides what a sequence takes: [`strip`] reads the
//! text with [`anstream`]'s, the VT500 state machine terminals follow, and
//! keeps what it prints. Removed:
//!
//! - an escape sequence: a control sequence (`ESC [`, CSI), a command string
//!   (`ESC ]` OSC, `ESC P` DCS, `ESC X` SOS, `ESC ^` PM, `ESC _` APC), or any
//!   other escape, read as a terminal reads it: a control inside one takes
//!   effect, CAN or SUB cancels it, and an `ESC` starts the next;
//! - every other C0 control but tab, line feed, form feed, and carriage
//!   return, which a terminal shows as space and line breaks; DEL; and each C1
//!   control (U+0080–U+009F).
//!
//! Each run of removed characters counts once.
//!
//! The parser starts afresh on each line, and a line break always stays: a
//! terminal would let an unterminated command string swallow on, but text
//! detection reads and no one sees costs at most a redaction, where a stray
//! `ESC ]` taking the rest of a file loses it. A C1 control goes alone, never
//! starting a sequence: a terminal reading UTF-8 does not start one there, and
//! one is what Windows-1252 text read as Latin-1 leaves for `ž` or `Ÿ`.

use std::borrow::Cow;

use anstream::adapter::strip_str;

/// What stayed once terminal control sequences were stripped, and how many
/// runs of them went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Stripped<T> {
    /// What stayed: a text, or a handler over one.
    pub(crate) kept: T,
    /// How many runs of control sequences and characters were removed.
    pub(crate) removed: usize,
}

/// `text` without its terminal control sequences and control characters;
/// borrowed when there are none.
pub(crate) fn strip(text: &str) -> Stripped<Cow<'_, str>> {
    if !text.chars().any(is_control) {
        return Stripped {
            kept: Cow::Borrowed(text),
            removed: 0,
        };
    }
    let mut out = String::with_capacity(text.len());
    let mut removed = 0;
    for line in text.split_inclusive(['\n', '\r']) {
        let body = line.trim_end_matches(['\n', '\r']);
        // Where the last kept piece ended, in `body`.
        let mut kept = 0;
        for piece in strip_str(body) {
            let at = piece.as_ptr() as usize - body.as_ptr() as usize;
            removed += usize::from(at > kept);
            kept = at + piece.len();
            for c in piece.chars() {
                if is_c1(c) {
                    removed += 1;
                } else {
                    out.push(c);
                }
            }
        }
        removed += usize::from(kept < body.len());
        out.push_str(&line[body.len()..]);
    }
    Stripped {
        kept: Cow::Owned(out),
        removed,
    }
}

/// [`strip`] over an owned `text`, handed back unchanged when it holds no
/// control.
#[cfg(any(feature = "txt", feature = "csv"))]
pub(crate) fn strip_owned(text: String) -> Stripped<String> {
    let Stripped { kept, removed } = strip(&text);
    let kept = match kept {
        Cow::Owned(kept) => kept,
        Cow::Borrowed(_) => text,
    };
    Stripped { kept, removed }
}

/// Whether `c` is a control a terminal obeys: a C0 control but tab, line
/// feed, form feed, and carriage return, DEL, or a C1 control.
fn is_control(c: char) -> bool {
    c.is_control() && !matches!(c, '\t' | '\n' | '\x0c' | '\r')
}

/// Whether `c` is a C1 control, which the parser, reading UTF-8, prints.
fn is_c1(c: char) -> bool {
    matches!(c, '\u{80}'..='\u{9f}')
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `text` stripped, and how many runs went.
    fn stripped(text: &str) -> (String, usize) {
        let Stripped { kept, removed } = strip(text);
        (kept.into_owned(), removed)
    }

    #[test]
    fn plain_text_is_borrowed() {
        let Stripped { kept, removed } = strip("Jane Roe\tjane@example.com\r\n");
        assert!(matches!(kept, Cow::Borrowed(_)));
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
    /// CAN cancels it, an `ESC` starts the next, and a line break ends it.
    #[test]
    fn controls_inside_a_sequence_act() {
        assert_eq!(stripped("Ja\u{1b}[\0mne"), ("Jane".into(), 1));
        assert_eq!(stripped("Ja\u{1b}(\u{7}Bne"), ("Jane".into(), 1));
        assert_eq!(stripped("Ja\u{1b}[1\u{18}mne"), ("Jamne".into(), 1));
        assert_eq!(stripped("Ja\u{1b}[1\u{1b}[mne"), ("Jane".into(), 1));
        assert_eq!(stripped("Ja\u{1b}[1\n;2mne"), ("Ja\n;2mne".into(), 1));
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
        assert_eq!(stripped("\u{1b}]0;title\u{1b}[mJane"), ("Jane".into(), 1));
        // Unterminated, it ends with its line.
        assert_eq!(stripped("Jane\u{1b}]0;Roe"), ("Jane".into(), 1));
        assert_eq!(
            stripped("Jane\u{1b}]0;x\r\nRoe\nDoe"),
            ("Jane\r\nRoe\nDoe".into(), 1)
        );
    }

    #[test]
    fn other_escapes_and_controls_go() {
        // A character-set designation, a reset, a backspace, and a bell and
        // DEL, one run.
        assert_eq!(stripped("\u{1b}(BJane"), ("Jane".into(), 1));
        assert_eq!(stripped("\u{1b}cJane"), ("Jane".into(), 1));
        assert_eq!(stripped("Janx\u{8}e\u{7}\u{7f}"), ("Janxe".into(), 2));
        // A lone ESC at the end, and one before a control.
        assert_eq!(stripped("Jane\u{1b}"), ("Jane".into(), 1));
        assert_eq!(stripped("Jane\u{1b}\u{1b}[m"), ("Jane".into(), 1));
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
