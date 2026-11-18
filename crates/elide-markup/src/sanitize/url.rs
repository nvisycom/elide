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
            .any(|word| {
                REMOTE_SCHEMES
                    .iter()
                    .any(|scheme| scheme_of(word).starts_with(scheme))
            })
    } else {
        is_remote(value)
    }
}

/// Whether the URL `value` reaches a server.
pub(super) fn is_remote(value: &str) -> bool {
    let url = normalized_url(value);
    REMOTE_SCHEMES.iter().any(|scheme| url.starts_with(scheme))
}

/// Whether the CSS `css` loads anything from a server, through a `url(...)` or
/// an `@import`.
pub(super) fn css_loads_remote(css: &str) -> bool {
    let css = css.to_ascii_lowercase();
    let urls = css.split("url(").skip(1).map(|rest| {
        rest.split(')')
            .next()
            .unwrap_or_default()
            .trim_matches(|c: char| c.is_whitespace() || c == '"' || c == '\'')
    });
    let imports = css
        .split("@import")
        .skip(1)
        .map(|rest| rest.trim_start().trim_start_matches(['"', '\'']));
    urls.chain(imports).any(is_remote)
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
