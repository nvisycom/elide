//! [`edit`]: rewrite an XML part's elements and attributes by namespace-aware
//! [`Rules`], leaving every byte outside an edit as it was.
//!
//! An OOXML part is sanitized by cutting byte ranges out of its source: an
//! element removed with its subtree, an element unwrapped (its tags gone, its
//! children kept), or an attribute removed from a start tag. Nothing is
//! re-serialized, so formatting, namespace declarations, and every untouched
//! element round-trip exactly. Element and attribute names are matched by
//! namespace URI, never by prefix, so a part that binds `w:` to an unusual
//! prefix is still recognized.

use std::borrow::Cow;
use std::ops::Range;

use elide_core::{Error, ErrorKind, Result};
use quick_xml::NsReader;
use quick_xml::events::{BytesStart, Event};
use quick_xml::name::{Namespace, ResolveResult};

/// A namespace-qualified element or attribute name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Name<'a> {
    /// The bound namespace URI, or `None` when the name is in no namespace.
    pub(crate) namespace: Option<&'a str>,
    /// The local name.
    pub(crate) local: &'a str,
}

impl Name<'_> {
    /// Whether this is `local` in `namespace`.
    pub(crate) fn is(&self, namespace: &str, local: &str) -> bool {
        self.namespace == Some(namespace) && self.local == local
    }

    /// Whether this is in `namespace`.
    pub(crate) fn in_namespace(&self, namespace: &str) -> bool {
        self.namespace == Some(namespace)
    }
}

/// One attribute of a start tag: its name and its value as written (still
/// escaped).
#[derive(Debug, Clone)]
pub(crate) struct Attribute<'a> {
    pub(crate) name: Name<'a>,
    pub(crate) value: &'a str,
    /// The attribute's byte range in the part, its leading whitespace included.
    range: Range<usize>,
}

/// What an edit does with an element.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Action {
    /// Keep the element (its attributes may still be removed).
    Keep,
    /// Remove the element with everything inside it.
    Remove,
    /// Remove the element's own tags, keeping its children in its place.
    Unwrap,
}

/// The decisions an [`edit`] asks of a format, for each element it meets.
pub(crate) trait Rules {
    /// What to do with the element `name` carrying `attributes`.
    fn element(&self, name: Name<'_>, attributes: &[Attribute<'_>]) -> Action;

    /// Whether to remove `attribute` from a kept element `element`.
    fn removes_attribute(&self, element: Name<'_>, attribute: &Attribute<'_>) -> bool;
}

/// A namespace-qualified name to match: `(namespace URI, local name)`.
pub(crate) type QName = (&'static str, &'static str);

/// How an attribute's local name is matched.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Local {
    /// Exactly this local name.
    Exact(&'static str),
    /// Any local name starting with this prefix.
    Prefix(&'static str),
    /// Any local name.
    Any,
}

/// [`Rules`] declared as name lists: elements to remove, elements to unwrap,
/// and attributes (by namespace and local-name match) to remove from every
/// element kept.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Declared {
    pub(crate) remove: &'static [QName],
    pub(crate) unwrap: &'static [QName],
    pub(crate) attributes: &'static [(&'static str, Local)],
}

impl Rules for Declared {
    fn element(&self, name: Name<'_>, _attributes: &[Attribute<'_>]) -> Action {
        let matches = |list: &[QName]| {
            list.iter()
                .any(|&(namespace, local)| name.is(namespace, local))
        };
        if matches(self.remove) {
            Action::Remove
        } else if matches(self.unwrap) {
            Action::Unwrap
        } else {
            Action::Keep
        }
    }

    fn removes_attribute(&self, _element: Name<'_>, attribute: &Attribute<'_>) -> bool {
        self.attributes.iter().any(|&(namespace, local)| {
            attribute.name.in_namespace(namespace)
                && match local {
                    Local::Exact(exact) => attribute.name.local == exact,
                    Local::Prefix(prefix) => attribute.name.local.starts_with(prefix),
                    Local::Any => true,
                }
        })
    }
}

/// How deep elements may nest before a part is refused. Real OOXML nests a few
/// dozen levels at most (a table in a text box in a table); bounding it keeps
/// every reader of the part, ours and the consumer's, within a bounded stack.
const MAX_DEPTH: usize = 256;

/// Check that `raw` is a part this crate can read safely: well-formed, nested
/// within [`MAX_DEPTH`], and with no document type declaration.
///
/// # Errors
///
/// As [`edit`].
pub(crate) fn check(raw: &str) -> Result<()> {
    edit(raw, &Unchanged).map(|_| ())
}

/// Rules that change nothing, for a structural [`check`].
struct Unchanged;

impl Rules for Unchanged {
    fn element(&self, _name: Name<'_>, _attributes: &[Attribute<'_>]) -> Action {
        Action::Keep
    }

    fn removes_attribute(&self, _element: Name<'_>, _attribute: &Attribute<'_>) -> bool {
        false
    }
}

/// `raw` edited by `rules`, or `None` when the rules change nothing.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`] if `raw` is not well-formed XML, nests past
/// [`MAX_DEPTH`], or declares a document type: a DTD's entities expand into
/// text no extraction reads, and OOXML never uses one.
pub(crate) fn edit(raw: &str, rules: &dyn Rules) -> Result<Option<String>> {
    let mut reader = NsReader::from_str(raw);
    // quick-xml reports positions past a leading BOM, but the ranges index the
    // original bytes.
    let bom = if raw.starts_with('\u{feff}') { 3 } else { 0 };
    let mut last = bom;
    let mut cuts: Vec<Range<usize>> = Vec::new();
    // The open elements' actions, to know what an end tag closes.
    let mut open: Vec<Action> = Vec::new();
    // A removed element in progress: its start byte and the open depth inside it.
    let mut removing: Option<(usize, usize)> = None;

    loop {
        let (namespace, event) = {
            let (resolved, event) = reader.read_resolved_event().map_err(malformed)?;
            (bound(&resolved), event)
        };
        let span = last..reader.buffer_position() as usize + bom;
        last = span.end;

        if let Some((start, depth)) = &mut removing {
            match event {
                Event::Start(_) => *depth += 1,
                Event::End(_) => {
                    *depth -= 1;
                    if *depth == 0 {
                        cuts.push(*start..span.end);
                        removing = None;
                    }
                }
                Event::Eof => return Err(malformed_message("unclosed element")),
                _ => {}
            }
            continue;
        }

        match event {
            Event::Eof => break,
            Event::DocType(_) => return Err(malformed_message("a document type declaration")),
            Event::Start(ref e) | Event::Empty(ref e) => {
                let is_start = matches!(event, Event::Start(_));
                if is_start && open.len() >= MAX_DEPTH {
                    return Err(malformed_message("elements nest too deep"));
                }
                let local = e.local_name();
                let name = Name {
                    namespace: namespace.as_deref(),
                    local: local.into_inner(),
                };
                let owned = attributes(&reader, raw, e)?;
                let attributes: Vec<Attribute<'_>> =
                    owned.iter().map(OwnedAttribute::view).collect();
                match rules.element(name, &attributes) {
                    Action::Remove if is_start => removing = Some((span.start, 1)),
                    Action::Remove | Action::Unwrap => {
                        cuts.push(span.clone());
                        if is_start {
                            open.push(Action::Unwrap);
                        }
                    }
                    Action::Keep => {
                        cuts.extend(
                            attributes
                                .iter()
                                .filter(|attribute| rules.removes_attribute(name, attribute))
                                .map(|attribute| attribute.range.clone()),
                        );
                        if is_start {
                            open.push(Action::Keep);
                        }
                    }
                }
            }
            Event::End(_) => {
                let closed = open.pop();
                if closed == Some(Action::Unwrap) {
                    cuts.push(span);
                }
            }
            _ => {}
        }
    }

    if cuts.is_empty() {
        return Ok(None);
    }
    // An attribute cut never overlaps another cut: an element removed whole is
    // cut before its attributes are looked at.
    cuts.sort_by_key(|range| range.start);
    let mut out = String::with_capacity(raw.len());
    let mut cursor = 0;
    for cut in cuts {
        out.push_str(&raw[cursor..cut.start]);
        cursor = cut.end;
    }
    out.push_str(&raw[cursor..]);
    Ok(Some(out))
}

/// An attribute with its namespace resolved into an owned string, so it
/// outlives the reader borrow the resolution took.
struct OwnedAttribute<'a> {
    namespace: Option<String>,
    local: &'a str,
    value: &'a str,
    range: Range<usize>,
}

impl OwnedAttribute<'_> {
    fn view(&self) -> Attribute<'_> {
        Attribute {
            name: Name {
                namespace: self.namespace.as_deref(),
                local: self.local,
            },
            value: self.value,
            range: self.range.clone(),
        }
    }
}

/// The namespace URI a resolved name is bound to, if any.
fn bound(resolved: &ResolveResult<'_>) -> Option<String> {
    match resolved {
        ResolveResult::Bound(Namespace(uri)) => Some((*uri).to_owned()),
        _ => None,
    }
}

/// Every attribute of `e`, resolved and located in `raw`.
fn attributes<'a>(
    reader: &NsReader<&[u8]>,
    raw: &'a str,
    e: &BytesStart<'a>,
) -> Result<Vec<OwnedAttribute<'a>>> {
    let base = raw.as_ptr() as usize;
    let offset = |slice: &str| {
        (slice.as_ptr() as usize)
            .checked_sub(base)
            .filter(|&at| at + slice.len() <= raw.len())
            .ok_or_else(|| malformed_message("attribute outside the part"))
    };
    let mut out = Vec::new();
    for attr in e.attributes() {
        let attr = attr.map_err(|e| malformed(e.into()))?;
        let key = attr.key.into_inner();
        let key_start = offset(key)?;
        let Cow::Borrowed(value) = attr.value else {
            return Err(malformed_message("attribute without a source"));
        };
        let value_start = offset(value)?;
        let value_end = value_start + value.len();
        let (resolved, local) = reader.resolver().resolve_attribute(attr.key);
        let start = raw[..key_start]
            .trim_end_matches(|c: char| c.is_ascii_whitespace())
            .len();
        out.push(OwnedAttribute {
            namespace: bound(&resolved),
            local: &raw[key_start + key.len() - local.into_inner().len()..key_start + key.len()],
            value: &raw[value_start..value_end],
            // Past the closing quote.
            range: start..value_end + 1,
        });
    }
    Ok(out)
}

fn malformed(e: quick_xml::Error) -> Error {
    Error::new(
        ErrorKind::MalformedInput,
        format!("malformed part XML: {e}"),
    )
}

fn malformed_message(message: &str) -> Error {
    Error::new(
        ErrorKind::MalformedInput,
        format!("malformed part XML: {message}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

    /// Remove `w:del`, unwrap `w:ins`, and drop `w:rsid*` attributes.
    struct Revisions;

    impl Rules for Revisions {
        fn element(&self, name: Name<'_>, _attributes: &[Attribute<'_>]) -> Action {
            if name.is(W, "del") {
                Action::Remove
            } else if name.is(W, "ins") {
                Action::Unwrap
            } else {
                Action::Keep
            }
        }

        fn removes_attribute(&self, _element: Name<'_>, attribute: &Attribute<'_>) -> bool {
            attribute.name.in_namespace(W) && attribute.name.local.starts_with("rsid")
        }
    }

    #[test]
    fn removes_unwraps_and_strips_attributes_byte_faithfully() {
        let raw = r#"<w:p xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" w:rsidR="00AB"><w:ins w:author="Jane"><w:r><w:t>kept</w:t></w:r></w:ins><w:del w:author="Jane"><w:r><w:delText>gone</w:delText></w:r></w:del><w:r w:rsidRPr="1"><w:t xml:space="preserve"> tail</w:t></w:r></w:p>"#;
        let out = edit(raw, &Revisions).unwrap().unwrap();
        assert_eq!(
            out,
            r#"<w:p xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:r><w:t>kept</w:t></w:r><w:r><w:t xml:space="preserve"> tail</w:t></w:r></w:p>"#
        );
    }

    #[test]
    fn matches_by_namespace_not_prefix() {
        let raw = r#"<x:del xmlns:x="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><x:r/></x:del><del>kept</del>"#;
        assert_eq!(edit(raw, &Revisions).unwrap().unwrap(), "<del>kept</del>");
    }

    #[test]
    fn an_untouched_part_is_unchanged() {
        assert!(
            edit(r#"<w:p xmlns:w="urn:other"><w:del/></w:p>"#, &Revisions)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn check_refuses_a_document_type_and_runaway_nesting() {
        assert!(check(r#"<!DOCTYPE d [<!ENTITY x "Jane">]><d>&x;</d>"#).is_err());
        let deep = format!(
            "{}{}",
            "<a>".repeat(MAX_DEPTH + 1),
            "</a>".repeat(MAX_DEPTH + 1)
        );
        assert!(check(&deep).is_err());
        assert!(check("<a><b/></a>").is_ok());
    }
}
