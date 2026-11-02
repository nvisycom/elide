//! [`HtmlTagExt`]: reading an HTML tag's name and attributes the way a
//! browser does, as an extension of quick-xml's tag events.
//!
//! quick-xml finds where a tag starts and ends, but reads its name and
//! attributes by XML's rules, which differ from the HTML tokenizer's
//! (WHATWG HTML §13.2.5) in ways a sanitizer cannot afford: in
//! `<a/onclick=b>` a browser reads an `a` with an `onclick` handler, where XML
//! reads one tag named `a/onclick=b`. This module re-reads the tag's bytes
//! with the tokenizer's tag name and attribute states, so every HTML reader
//! sees the name and attributes a browser sees.
//!
//! Where a tag, comment, or other construct starts and ends is still
//! quick-xml's call; the sanitizer refuses the constructs a browser ends
//! earlier than quick-xml does, and a tag a browser ends early is refused
//! here.

use std::ops::Range;

use elide_core::{Error, Result};
use quick_xml::events::{BytesEnd, BytesStart};

/// An HTML tag as a browser reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HtmlTag<'a> {
    /// The tag name as written; a browser matches it lowercased.
    pub(crate) name: &'a str,
    /// The attributes, in source order.
    pub(crate) attributes: Vec<HtmlAttribute<'a>>,
}

/// One attribute of an HTML tag, borrowed from the tag as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HtmlAttribute<'a> {
    /// The name as written.
    pub(crate) name: &'a str,
    /// The value as written, between its quotes; `None` for an attribute
    /// written without one (`<input disabled>`).
    pub(crate) value: Option<&'a str>,
    /// The whole attribute, from its name to the end of its value (closing
    /// quote included).
    pub(crate) source: &'a str,
}

/// Reads a quick-xml tag event the way a browser reads the HTML tag.
pub(crate) trait HtmlTagExt {
    /// The tag's name and attributes as a browser reads them, borrowed from
    /// `raw`, the document the event was read from.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput)
    /// when the event was not read from `raw`, or when a browser would end the
    /// tag before quick-xml does: the two would disagree on everything after
    /// it, so the document is refused rather than read differently from how it
    /// renders.
    fn html5_tag<'a>(&self, raw: &'a str) -> Result<HtmlTag<'a>>;
}

impl HtmlTagExt for BytesStart<'_> {
    fn html5_tag<'a>(&self, raw: &'a str) -> Result<HtmlTag<'a>> {
        let mut text = place(raw, self)?;
        // quick-xml leaves the `/` of an empty tag's `/>` out of its bytes,
        // but a browser reads it: as the end of an unquoted value, or as the
        // self-closing flag.
        if raw[text.end..].starts_with("/>") {
            text.end += 1;
        }
        read_tag(&raw[text])
    }
}

impl HtmlTagExt for BytesEnd<'_> {
    /// An end tag's attributes are ignored by a browser, so only its name is
    /// read.
    fn html5_tag<'a>(&self, raw: &'a str) -> Result<HtmlTag<'a>> {
        let text = &raw[place(raw, self)?];
        Ok(HtmlTag {
            name: &text[..read_name(text)],
            attributes: Vec::new(),
        })
    }
}

/// Where the tag bytes `tag`, borrowed from `raw`, sit in it.
fn place(raw: &str, tag: &str) -> Result<Range<usize>> {
    (tag.as_ptr() as usize)
        .checked_sub(raw.as_ptr() as usize)
        .filter(|&at| at + tag.len() <= raw.len())
        .map(|at| at..at + tag.len())
        .ok_or_else(|| Error::malformed("an HTML tag was not read from its document"))
}

/// Whether `byte` is HTML whitespace between the parts of a tag.
fn is_space(byte: u8) -> bool {
    matches!(byte, b'\t' | b'\n' | b'\x0C' | b' ')
}

/// Where the tag name at the start of `text` ends: at whitespace, `/`, or the
/// end of the tag.
fn read_name(text: &str) -> usize {
    text.bytes()
        .position(|byte| is_space(byte) || byte == b'/' || byte == b'>')
        .unwrap_or(text.len())
}

/// The tag `text` (between `<` and `>`, as quick-xml found it) read by the
/// tokenizer's tag states.
fn read_tag(text: &str) -> Result<HtmlTag<'_>> {
    let bytes = text.as_bytes();
    let mut at = read_name(text);
    let name = &text[..at];
    let mut attributes = Vec::new();
    loop {
        // Before attribute name: skip whitespace, and a `/` not ending the
        // tag reads like whitespace.
        while at < bytes.len() && (is_space(bytes[at]) || bytes[at] == b'/') {
            at += 1;
        }
        if at == bytes.len() {
            break;
        }
        if bytes[at] == b'>' {
            return Err(early_end(text));
        }
        // Attribute name: up to whitespace, `/`, `>`, or `=`; a leading `=` is
        // part of the name.
        let name_start = at;
        at += 1;
        while at < bytes.len() && !matches!(bytes[at], b'/' | b'>' | b'=') && !is_space(bytes[at]) {
            at += 1;
        }
        let name_end = at;
        // After attribute name: skip whitespace; an `=` brings a value.
        let mut look = at;
        while look < bytes.len() && is_space(bytes[look]) {
            look += 1;
        }
        if look == bytes.len() || bytes[look] != b'=' {
            attributes.push(attribute(text, name_start..name_end, None, name_end));
            continue;
        }
        // Before attribute value: skip whitespace.
        at = look + 1;
        while at < bytes.len() && is_space(bytes[at]) {
            at += 1;
        }
        let (value, end) = match bytes.get(at) {
            // A missing value: `<a title=>` reads as an empty one.
            None => (at..at, at),
            Some(b'>') => return Err(early_end(text)),
            Some(&quote @ (b'"' | b'\'')) => {
                let close = text[at + 1..]
                    .find(quote as char)
                    .map(|i| at + 1 + i)
                    .ok_or_else(|| Error::malformed("an HTML attribute value is not closed"))?;
                (at + 1..close, close + 1)
            }
            Some(_) => {
                let start = at;
                while at < bytes.len() && !is_space(bytes[at]) && bytes[at] != b'>' {
                    at += 1;
                }
                (start..at, at)
            }
        };
        attributes.push(attribute(text, name_start..name_end, Some(value), end));
        at = end;
    }
    Ok(HtmlTag { name, attributes })
}

/// The attribute of `text` named at `name`, with its `value` range, ending at
/// byte `end`.
fn attribute(
    text: &str,
    name: Range<usize>,
    value: Option<Range<usize>>,
    end: usize,
) -> HtmlAttribute<'_> {
    HtmlAttribute {
        name: &text[name.clone()],
        value: value.map(|value| &text[value]),
        source: &text[name.start..end],
    }
}

/// The error for a tag a browser ends at a `>` quick-xml read as inside it.
fn early_end(text: &str) -> Error {
    Error::malformed(format!(
        "a browser ends the HTML tag `<{text}>` early, at a `>` inside it"
    ))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use html5ever::tendril::StrTendril;
    use html5ever::tokenizer::{
        BufferQueue, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer, TokenizerOpts,
    };
    use quick_xml::Reader;
    use quick_xml::events::Event;

    use super::*;

    /// A tag's attributes as `(name, value)` pairs.
    type Pairs = Vec<(String, Option<String>)>;

    /// The name and attributes of the first start tag of `raw`.
    fn read(raw: &str) -> Result<(String, Pairs)> {
        let mut reader = Reader::from_str(raw);
        loop {
            match reader.read_event().expect("quick-xml reads the tag") {
                Event::Start(e) | Event::Empty(e) => {
                    let tag = e.html5_tag(raw)?;
                    let attributes = tag
                        .attributes
                        .iter()
                        .map(|attr| (attr.name.to_owned(), attr.value.map(str::to_owned)))
                        .collect();
                    return Ok((tag.name.to_owned(), attributes));
                }
                Event::Eof => panic!("no start tag in {raw:?}"),
                _ => {}
            }
        }
    }

    fn attrs(pairs: &[(&str, Option<&str>)]) -> Pairs {
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), value.map(str::to_owned)))
            .collect()
    }

    #[test]
    fn a_slash_between_attributes_reads_like_whitespace() {
        let (name, attributes) = read("<a/onclick=b>t</a>").unwrap();
        assert_eq!(name, "a");
        assert_eq!(attributes, attrs(&[("onclick", Some("b"))]));
        let (_, attributes) = read(r#"<a title="x"/onclick=b>t</a>"#).unwrap();
        assert_eq!(
            attributes,
            attrs(&[("title", Some("x")), ("onclick", Some("b"))])
        );
    }

    #[test]
    fn values_may_be_unquoted_missing_or_absent() {
        let (_, attributes) = read("<input value=jane@example.com disabled title=>").unwrap();
        assert_eq!(
            attributes,
            attrs(&[
                ("value", Some("jane@example.com")),
                ("disabled", None),
                ("title", Some("")),
            ])
        );
    }

    #[test]
    fn odd_names_and_adjacent_values_read_as_a_browser_reads_them() {
        let (_, attributes) = read(r#"<a ="x" title=="Jane" b='a'"c" d=x=y>t</a>"#).unwrap();
        assert_eq!(
            attributes,
            attrs(&[
                ("=\"x\"", None),
                ("title", Some("=\"Jane\"")),
                ("b", Some("a")),
                ("\"c\"", None),
                ("d", Some("x=y")),
            ])
        );
    }

    #[test]
    fn spaces_around_the_equals_sign_are_skipped() {
        let (_, attributes) = read("<a title = 'Jane Roe' >t</a>").unwrap();
        assert_eq!(attributes, attrs(&[("title", Some("Jane Roe"))]));
    }

    #[test]
    fn the_source_spans_the_whole_attribute() {
        let raw = r#"<a title="x" onclick=b disabled>"#;
        let mut reader = Reader::from_str(raw);
        let Ok(Event::Start(e)) = reader.read_event() else {
            panic!("a start tag");
        };
        let tag = e.html5_tag(raw).unwrap();
        let sources: Vec<&str> = tag.attributes.iter().map(|attr| attr.source).collect();
        assert_eq!(sources, [r#"title="x""#, "onclick=b", "disabled"]);
    }

    #[test]
    fn a_tag_a_browser_ends_early_is_refused() {
        // quick-xml reads the quote as opening a value and runs the tag on to
        // the next `>` after its close; a browser ends it at the first `>`.
        let raw = r#"<a b"c>text<i d"e>t</i>"#;
        assert!(read(raw).is_err());
    }

    #[test]
    fn an_end_tag_name_ends_at_a_slash_or_space() {
        let raw = "<script>x</SCRIPT/x>";
        let mut reader = Reader::from_str(raw);
        reader.config_mut().check_end_names = false;
        loop {
            if let Event::End(e) = reader.read_event().unwrap() {
                assert_eq!(e.html5_tag(raw).unwrap().name, "SCRIPT");
                break;
            }
        }
    }

    /// The first start tag html5ever's tokenizer reads: its name and its
    /// attributes as `(name, value)`, names lowercased and the first of a
    /// repeated name kept.
    #[derive(Default)]
    struct FirstStartTag(RefCell<Option<ReferenceTag>>);

    /// A tag as the reference reads it: its name and `(name, value)` pairs.
    type ReferenceTag = (String, Vec<(String, String)>);

    impl TokenSink for FirstStartTag {
        type Handle = ();

        fn process_token(&self, token: Token, _line: u64) -> TokenSinkResult<()> {
            if let Token::TagToken(tag) = token
                && tag.kind == TagKind::StartTag
                && self.0.borrow().is_none()
            {
                let attributes = tag
                    .attrs
                    .iter()
                    .map(|attr| (attr.name.local.to_string(), attr.value.to_string()))
                    .collect();
                *self.0.borrow_mut() = Some((tag.name.to_string(), attributes));
            }
            TokenSinkResult::Continue
        }
    }

    /// The first start tag of `raw` as html5ever reads it, the reference.
    fn reference(raw: &str) -> ReferenceTag {
        let tokenizer = Tokenizer::new(FirstStartTag::default(), TokenizerOpts::default());
        let input = BufferQueue::default();
        input.push_back(StrTendril::from_slice(raw));
        let _ = tokenizer.feed(&input);
        tokenizer.end();
        tokenizer
            .sink
            .0
            .take()
            .unwrap_or_else(|| panic!("no start tag in {raw:?}"))
    }

    #[test]
    fn every_tag_reads_as_the_reference_tokenizer_reads_it() {
        // Tags quick-xml accepts, with no character reference (html5ever
        // decodes values, this reader keeps them raw).
        let corpus = [
            "<a title=JaneRoe href=x>t</a>",
            "<a/onclick=b>t</a>",
            r#"<a title="x"/onclick=b>t</a>"#,
            r#"<a ="x" title="Jane">t</a>"#,
            r#"<a title=="Jane" onclick="x()">t</a>"#,
            r#"<a title="a"onclick="b">t</a>"#,
            r#"<a "x" onclick="b">t</a>"#,
            "<a title= onclick=b>t</a>",
            "<a = onclick=b>t</a>",
            "<a title=x=y onclick=b>t</a>",
            "<a title=`x` onclick=b>t</a>",
            r#"<a title='a'"b" onclick=c>t</a>"#,
            "<input value=jane@example.com disabled title=>",
            "<A HREF=X ONCLICK=Y>t</A>",
            "<a\ttitle\n=\x0C'Jane Roe' >t</a>",
            r#"<a title="Jane" title="Roe">t</a>"#,
            r#"<img src="x.png" alt='Jane Roe'/>"#,
            "<div/hidden/>t</div>",
            r#"<p//class="x"//>t</p>"#,
            "<p//class=x//>t</p>",
            "<img alt=Jane/>",
        ];
        for raw in corpus {
            let ours = ours(raw).unwrap_or_else(|| panic!("{raw} is refused"));
            assert_eq!(ours, reference(raw), "{raw}");
        }
    }

    #[test]
    fn generated_tags_read_as_the_reference_tokenizer_reads_them() {
        // The pieces a tag's name and attributes are built from: names, the
        // characters that separate or quote them, and mixed case.
        const PIECES: &[&str] = &[
            "a", "B", "x", "on", "=", "\"", "'", "`", "/", " ", "\t", "\n", "\x0C",
        ];
        // A fixed xorshift sequence, so a failure reproduces.
        let mut state: u64 = 0x5EED_1234_ABCD_9876;
        let mut next = |bound: usize| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state % bound as u64) as usize
        };
        let mut compared = 0;
        for _ in 0..20_000 {
            let mut raw = String::from("<p");
            for _ in 0..next(14) {
                raw.push_str(PIECES[next(PIECES.len())]);
            }
            raw.push_str(">t");
            // Only a tag quick-xml reads whole and this reader does not refuse
            // is compared.
            let Some(ours) = ours(&raw) else {
                continue;
            };
            assert_eq!(ours, reference(&raw), "{raw:?}");
            compared += 1;
        }
        assert!(compared > 5_000, "only {compared} generated tags compared");
    }

    /// The first start tag of `raw` as this reader reads it, in the
    /// reference's terms (names lowercased, the first of a repeated name kept,
    /// a missing value empty); `None` when quick-xml cannot read the tag or
    /// this reader refuses it.
    fn ours(raw: &str) -> Option<ReferenceTag> {
        let mut reader = Reader::from_str(raw);
        let (Event::Start(e) | Event::Empty(e)) = reader.read_event().ok()? else {
            return None;
        };
        let tag = e.html5_tag(raw).ok()?;
        let mut attributes: Vec<(String, String)> = Vec::new();
        for attr in &tag.attributes {
            let name = attr.name.to_ascii_lowercase();
            if !attributes.iter().any(|(seen, _)| *seen == name) {
                attributes.push((name, attr.value.unwrap_or_default().to_owned()));
            }
        }
        Some((tag.name.to_ascii_lowercase(), attributes))
    }
}
