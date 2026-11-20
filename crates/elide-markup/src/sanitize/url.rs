//! How a browser reads a URL attribute, and whether it runs script or loads
//! from a server.

use crate::references::{CharacterReferences, decode_references};

/// URL schemes that run script when the URL is followed.
pub(super) const SCRIPT_SCHEMES: [&str; 3] = ["javascript:", "vbscript:", "data:text/html"];

/// URL schemes (and the scheme-relative form) that reach a server.
const REMOTE_SCHEMES: [&str; 5] = ["http:", "https:", "ftp:", "//", "\\\\"];

/// Whether the attribute `key`'s `value` loads anything from a server: a
/// `srcset` lists several URLs, every other attribute one.
///
/// A `srcset` is decoded before it is split, since a reference can stand for
/// the comma or space between candidates, and every word of it is checked:
/// a descriptor is never a remote URL, so nothing is missed by not telling
/// one from a URL.
pub(super) fn loads_remote(value: &str, key: &str) -> bool {
    if key == "srcset" {
        let (decoded, _) = decode_references(value, 0, CharacterReferences::Html);
        decoded
            .split(|c: char| c == ',' || c.is_ascii_whitespace())
            .any(reaches_server)
    } else {
        is_remote(value)
    }
}

/// Whether the URL attribute `value` reaches a server.
pub(super) fn is_remote(value: &str) -> bool {
    let (decoded, _) = decode_references(value, 0, CharacterReferences::Html);
    reaches_server(&decoded)
}

/// Whether the URL `decoded`, its character references already decoded,
/// reaches a server.
fn reaches_server(decoded: &str) -> bool {
    let url = scheme_of(decoded);
    REMOTE_SCHEMES.iter().any(|scheme| url.starts_with(scheme))
}

/// The CSS functions whose arguments a browser may load: `url()`, `src()`,
/// and `image-set()` (with its `-webkit-` form), whose strings are URLs too.
const CSS_LOADING_FUNCTIONS: [&str; 3] = ["url(", "src(", "image-set("];

/// Whether the CSS `css` loads anything from a server, through a loading
/// function or an `@import`.
///
/// `css` is the text CSS reads: the caller decodes the character references
/// of a style attribute or a foreign style sheet, and leaves those of an HTML
/// `<style>`, which a browser reads as written. CSS escapes and comments are
/// read here, both ways round and not at all, since a comment marker may
/// stand inside a string: a load any reading finds counts.
pub(super) fn css_loads_remote(css: &str) -> bool {
    let decoded = css_unescaped(css);
    [
        css_uncommented(css),
        css_uncommented(&decoded),
        css_unescaped(&css_uncommented(css)),
        decoded,
        css.to_owned(),
    ]
    .iter()
    .any(|css| css_reaches_server(&css.to_ascii_lowercase()))
}

/// Whether the CSS `css`, read as written and lowercased, names a remote URL in a loading
/// function or an `@import`.
fn css_reaches_server(css: &str) -> bool {
    let arguments = CSS_LOADING_FUNCTIONS.iter().flat_map(|function| {
        css.split(function).skip(1).flat_map(|rest| {
            rest.split(')')
                .next()
                .unwrap_or_default()
                .split(|c: char| c == ',' || c.is_whitespace())
                .map(|word| word.trim_matches(['"', '\'']))
        })
    });
    let imports = css
        .split("@import")
        .skip(1)
        .map(|rest| rest.trim_start().trim_start_matches(['"', '\'']));
    arguments.chain(imports).any(reaches_server)
}

/// The CSS `css` with its escapes read: a backslash and up to six hex digits
/// (and one whitespace after) stand for that code point, a backslash before
/// a line break continues the line, and before any other character stands
/// for it.
fn css_unescaped(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut chars = css.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        let mut hex = String::new();
        while hex.len() < 6
            && let Some(digit) = chars.next_if(char::is_ascii_hexdigit)
        {
            hex.push(digit);
        }
        if hex.is_empty() {
            match chars.next() {
                Some('\r') => {
                    chars.next_if_eq(&'\n');
                }
                Some('\n' | '\x0c') | None => {}
                Some(other) => out.push(other),
            }
            continue;
        }
        let code = u32::from_str_radix(&hex, 16).unwrap_or_default();
        out.push(
            char::from_u32(code)
                .filter(|c| *c != '\0')
                .unwrap_or('\u{fffd}'),
        );
        if chars.next_if_eq(&'\r').is_some() {
            chars.next_if_eq(&'\n');
        } else {
            chars.next_if(|c| matches!(c, ' ' | '\t' | '\n' | '\x0c'));
        }
    }
    out
}

/// The CSS `css` with its comments taken out, an unclosed one running to the
/// end.
fn css_uncommented(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(at) = rest.find("/*") {
        out.push_str(&rest[..at]);
        rest = rest[at + 2..]
            .find("*/")
            .map_or("", |end| &rest[at + 2 + end + 2..]);
    }
    out.push_str(rest);
    out
}

/// A URL attribute's `value` as a browser reads its scheme: character
/// references decoded, the tab and line breaks a URL parser drops removed, and
/// leading spaces and control characters trimmed, lowercased.
pub(super) fn normalized_url(value: &str) -> String {
    let (decoded, _) = decode_references(value, 0, CharacterReferences::Html);
    scheme_of(&decoded)
}

/// A decoded URL as a browser reads its scheme: the tab and line breaks a URL
/// parser drops removed, and leading spaces and control characters trimmed,
/// lowercased.
fn scheme_of(decoded: &str) -> String {
    decoded
        .chars()
        .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
        .skip_while(|c| *c <= ' ')
        .collect::<String>()
        .to_ascii_lowercase()
}
