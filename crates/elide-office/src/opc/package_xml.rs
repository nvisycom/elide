//! The allowlist of the package's own XML parts: the content-types manifest
//! and the relationships parts.
//!
//! Every other XML part is rebuilt from its schema; these two are written by
//! their own readers, so they are cut down to their OPC vocabulary here
//! instead: [`PackageMarkup`] cuts every element and attribute the vocabulary
//! does not name, and every processing instruction, and [`PackageText`] every
//! text, which neither part holds. What stays is the root, its entries with
//! their own attributes, and namespace declarations. Like every edit, both cut
//! only what they remove, so a clean part is kept byte for byte.

use std::ops::Range;

use elide_core::redaction::HiddenContent;
use elide_core::{Error, Result};
use quick_xml::escape::unescape;
use quick_xml::events::{BytesStart, Event};

use crate::opc::content_types::CONTENT_TYPES_PART;
use crate::opc::part::PartPath;
use crate::opc::xml_edit::{Edit, Edited, Skipped, SpannedReader, attribute_extent, splice};
use crate::opc::{malformed_xml, ns};

/// The vocabulary of a package part: its root, and each entry the root may
/// hold with the unqualified attributes it may carry.
#[derive(Debug)]
pub(crate) struct Vocabulary {
    /// The namespace of the root and its entries.
    namespace: &'static str,
    /// The root's local name.
    root: &'static str,
    /// Each entry's local name and attributes.
    entries: &'static [(&'static str, &'static [&'static str])],
}

/// The vocabulary of `[Content_Types].xml`.
pub(crate) static CONTENT_TYPES: Vocabulary = Vocabulary {
    namespace: ns::CONTENT_TYPES,
    root: "Types",
    entries: &[
        ("Default", &["Extension", "ContentType"]),
        ("Override", &["PartName", "ContentType"]),
    ],
};

/// The vocabulary of a relationships part.
pub(crate) static RELATIONSHIPS: Vocabulary = Vocabulary {
    namespace: ns::RELATIONSHIPS,
    root: "Relationships",
    entries: &[("Relationship", &["Id", "Type", "Target", "TargetMode"])],
};

/// The vocabulary of the package part `path`, or `None` when it is neither
/// the manifest nor a relationships part.
pub(crate) fn vocabulary_of(path: &PartPath) -> Option<&'static Vocabulary> {
    if path.as_str() == CONTENT_TYPES_PART {
        Some(&CONTENT_TYPES)
    } else if path.is_relationships() {
        Some(&RELATIONSHIPS)
    } else {
        None
    }
}

/// An entry the package part's readers take: its local name and its
/// allowed attributes with their decoded values.
pub(crate) type Entry = (&'static str, Vec<(&'static str, String)>);

/// The entries of the package part `xml` as [`PackageMarkup`] keeps them:
/// the root's children in `vocabulary`'s namespace (or none), with their
/// unqualified allowed attributes. Anything the allowlist cuts is not read,
/// so the part reads the same before and after it.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if
/// `xml` is not well-formed, naming the part as `what`.
pub(crate) fn entries(xml: &str, vocabulary: &Vocabulary, what: &str) -> Result<Vec<Entry>> {
    Ok(spanned_entries(xml, vocabulary, what)?
        .into_iter()
        .map(|(entry, _)| entry)
        .collect())
}

/// The package part `xml` without each entry whose `attribute`, read as
/// [`entries`] reads it, `drops`; byte for byte otherwise.
///
/// # Errors
///
/// As [`entries`].
pub(crate) fn without_entries(
    xml: &str,
    vocabulary: &Vocabulary,
    what: &str,
    attribute: &str,
    drops: impl Fn(&str) -> bool,
) -> Result<String> {
    let cuts = spanned_entries(xml, vocabulary, what)?
        .into_iter()
        .filter(|((_, attributes), _)| {
            attributes
                .iter()
                .any(|(key, value)| *key == attribute && drops(value))
        })
        .map(|(_, span)| (span, ""))
        .collect();
    Ok(splice(xml, cuts))
}

/// [`entries`], each with the span of its whole element.
fn spanned_entries(
    xml: &str,
    vocabulary: &Vocabulary,
    what: &str,
) -> Result<Vec<(Entry, Range<usize>)>> {
    let malformed = |e: &dyn std::fmt::Display| Error::malformed(format!("{what} malformed: {e}"));
    let mut reader = SpannedReader::new_ns(xml);
    let mut entries = Vec::new();
    // An entry whose element is still open, read to its end tag.
    let mut open: Option<(Entry, Skipped)> = None;
    let mut depth = 0usize;
    loop {
        let (namespace, event, span) = reader.read_resolved_event().map_err(|e| malformed(&e))?;
        if let Some((_, skipped)) = &mut open {
            if let Some(whole) = skipped.read(&event, span.end)? {
                let (entry, _) = open.take().expect("an open entry");
                entries.push((entry, whole));
                depth -= 1;
            }
            continue;
        }
        let (e, is_start) = match &event {
            Event::Eof => break,
            Event::Start(e) => (e, true),
            Event::Empty(e) => (e, false),
            Event::End(_) => {
                depth = depth.saturating_sub(1);
                continue;
            }
            _ => continue,
        };
        if depth == 1
            && let Some((name, allowed)) = vocabulary
                .entries
                .iter()
                .find(|(name, _)| *name == e.local_name().into_inner())
            && vocabulary.owns(namespace.as_deref())
        {
            let mut attributes = Vec::new();
            for attr in e.attributes() {
                let attr = attr.map_err(|e| malformed(&e))?;
                if attr.key.prefix().is_some() {
                    continue;
                }
                let Some(key) = allowed
                    .iter()
                    .find(|key| **key == attr.key.local_name().into_inner())
                else {
                    continue;
                };
                let value = unescape(&attr.value).map_err(|e| malformed(&e))?;
                attributes.push((*key, value.into_owned()));
            }
            let entry = (*name, attributes);
            if is_start {
                open = Some((entry, Skipped::new(span.start)));
                depth += 1;
            } else {
                entries.push((entry, span));
            }
            continue;
        }
        if is_start {
            depth += 1;
        }
    }
    Ok(entries)
}

impl Vocabulary {
    /// Whether an element in `namespace` is in this vocabulary's: its own, or
    /// none, as a part that never declares it is read.
    fn owns(&self, namespace: Option<&str>) -> bool {
        namespace.is_none_or(|namespace| namespace == self.namespace)
    }

    /// The attributes of the entry `local` in `namespace`, or `None` when the
    /// root holds no such entry.
    fn entry(&self, namespace: Option<&str>, local: &str) -> Option<&'static [&'static str]> {
        if !self.owns(namespace) {
            return None;
        }
        self.entries
            .iter()
            .find(|(name, _)| *name == local)
            .map(|(_, attributes)| *attributes)
    }
}

/// An element the markup edit keeps: the root, or one of its entries with the
/// attributes it may carry.
#[derive(Clone, Copy)]
enum Kept {
    Root,
    Entry(&'static [&'static str]),
}

/// The elements and attributes of a package part outside its [`Vocabulary`],
/// and its processing instructions.
pub(crate) struct PackageMarkup(pub(crate) &'static Vocabulary);

impl Edit for PackageMarkup {
    fn content(&self) -> Option<HiddenContent> {
        Some(HiddenContent::ApplicationData)
    }

    fn apply(&self, raw: &str) -> Result<Option<Edited>> {
        let vocabulary = self.0;
        let mut reader = SpannedReader::new_ns(raw);
        let mut cuts: Vec<(Range<usize>, &str)> = Vec::new();
        let mut open: Vec<Kept> = Vec::new();
        let mut removing: Option<Skipped> = None;
        loop {
            let (namespace, event, span) = reader.read_resolved_event().map_err(malformed_xml)?;
            if let Some(skipped) = &mut removing {
                if let Some(whole) = skipped.read(&event, span.end)? {
                    cuts.push((whole, ""));
                    removing = None;
                }
                continue;
            }
            match event {
                Event::Eof => break,
                Event::DocType(_) => return Err(malformed_xml("a document type declaration")),
                Event::PI(_) => cuts.push((span, "")),
                Event::Start(ref e) | Event::Empty(ref e) => {
                    let is_start = matches!(event, Event::Start(_));
                    let local = e.local_name().into_inner();
                    let kept = match open.last() {
                        None if vocabulary.owns(namespace.as_deref())
                            && local == vocabulary.root =>
                        {
                            Kept::Root
                        }
                        None => {
                            return Err(malformed_xml(format_args!(
                                "the root element is not `{}`",
                                vocabulary.root
                            )));
                        }
                        Some(Kept::Root) => match vocabulary.entry(namespace.as_deref(), local) {
                            Some(attributes) => Kept::Entry(attributes),
                            None => {
                                cut_element(&mut cuts, &mut removing, span, is_start);
                                continue;
                            }
                        },
                        // An entry holds nothing.
                        Some(Kept::Entry(_)) => {
                            cut_element(&mut cuts, &mut removing, span, is_start);
                            continue;
                        }
                    };
                    let allowed = match kept {
                        Kept::Root => &[][..],
                        Kept::Entry(attributes) => attributes,
                    };
                    cut_attributes(raw, e, allowed, &mut cuts)?;
                    if is_start {
                        open.push(kept);
                    }
                }
                Event::End(_) => {
                    open.pop();
                }
                _ => {}
            }
        }
        if cuts.is_empty() {
            return Ok(None);
        }
        let count = cuts.len();
        Ok(Some(Edited {
            text: splice(raw, cuts),
            count,
        }))
    }
}

/// Cut the element at `span`: an empty one now, a started one once
/// `removing` reads to its end.
fn cut_element(
    cuts: &mut Vec<(Range<usize>, &str)>,
    removing: &mut Option<Skipped>,
    span: Range<usize>,
    is_start: bool,
) {
    if is_start {
        *removing = Some(Skipped::new(span.start));
    } else {
        cuts.push((span, ""));
    }
}

/// Cut every attribute of `e` but a namespace declaration or an unqualified
/// one named in `allowed`, each with its leading whitespace.
fn cut_attributes(
    raw: &str,
    e: &BytesStart<'_>,
    allowed: &[&str],
    cuts: &mut Vec<(Range<usize>, &str)>,
) -> Result<()> {
    for attr in e.attributes() {
        let attr = attr.map_err(malformed_xml)?;
        let key = attr.key;
        let kept = key.as_namespace_binding().is_some()
            || (key.prefix().is_none()
                && allowed
                    .iter()
                    .any(|name| *name == key.local_name().into_inner()));
        if kept {
            continue;
        }
        let (extent, _) = attribute_extent(raw, &attr)?;
        cuts.push((extent, ""));
    }
    Ok(())
}

/// The text of a package part, which holds none: every non-blank text, CDATA
/// section, and character or entity reference.
pub(crate) struct PackageText;

impl Edit for PackageText {
    fn content(&self) -> Option<HiddenContent> {
        Some(HiddenContent::Undisplayed)
    }

    fn apply(&self, raw: &str) -> Result<Option<Edited>> {
        let mut reader = SpannedReader::new(raw);
        let mut cuts: Vec<(Range<usize>, &str)> = Vec::new();
        loop {
            let (event, span) = reader.read_event().map_err(malformed_xml)?;
            match event {
                Event::Eof => break,
                Event::Text(ref text) if !text.bytes().all(|byte| byte.is_ascii_whitespace()) => {
                    cuts.push((span, ""));
                }
                Event::CData(_) | Event::GeneralRef(_) => cuts.push((span, "")),
                _ => {}
            }
        }
        if cuts.is_empty() {
            return Ok(None);
        }
        let count = cuts.len();
        Ok(Some(Edited {
            text: splice(raw, cuts),
            count,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="main"/></Types>"#;

    const RELS: &str = r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="t/styles" Target="styles.xml"/><Relationship Id="rId2" Type="t/link" Target="https://example.com" TargetMode="External"/></Relationships>"#;

    /// `raw` with the markup edit, then the text edit, applied.
    fn allowlisted(raw: &str, vocabulary: &'static Vocabulary) -> String {
        let marked = PackageMarkup(vocabulary)
            .apply(raw)
            .unwrap()
            .map_or_else(|| raw.to_owned(), |edited| edited.text);
        PackageText
            .apply(&marked)
            .unwrap()
            .map_or(marked, |edited| edited.text)
    }

    #[test]
    fn clean_parts_are_unchanged() {
        assert!(
            PackageMarkup(&CONTENT_TYPES)
                .apply(MANIFEST)
                .unwrap()
                .is_none()
        );
        assert!(PackageText.apply(MANIFEST).unwrap().is_none());
        assert!(PackageMarkup(&RELATIONSHIPS).apply(RELS).unwrap().is_none());
        assert!(PackageText.apply(RELS).unwrap().is_none());
    }

    #[test]
    fn a_manifest_keeps_only_its_vocabulary() {
        let raw = r#"<?xml version="1.0"?><?vendor note?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types" xmlns:v="urn:vendor" v:owner="Jane"><v:note>secret</v:note><Default Extension="xml" ContentType="application/xml" v:by="Jane" Note="x"><Hidden/></Default>stray<Override PartName="/a.xml" ContentType="main"><![CDATA[cdata]]></Override><Other PartName="/b.xml"/>&amp;</Types>"#;
        let markup = PackageMarkup(&CONTENT_TYPES).apply(raw).unwrap().unwrap();
        assert_eq!(markup.count, 7);
        assert_eq!(
            allowlisted(raw, &CONTENT_TYPES),
            r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types" xmlns:v="urn:vendor"><Default Extension="xml" ContentType="application/xml"></Default><Override PartName="/a.xml" ContentType="main"></Override></Types>"#
        );
    }

    #[test]
    fn a_relationships_part_keeps_only_its_vocabulary() {
        let raw = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="t" Target="a.xml" Author="Jane"/><Extra Id="rId9"/>  <Relationship Id="rId2" Type="t" Target="b.xml">note</Relationship></Relationships>"#;
        assert_eq!(
            allowlisted(raw, &RELATIONSHIPS),
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="t" Target="a.xml"/>  <Relationship Id="rId2" Type="t" Target="b.xml"></Relationship></Relationships>"#
        );
    }

    #[test]
    fn an_entry_in_another_namespace_goes() {
        let raw = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships" xmlns:v="urn:vendor"><v:Relationship Id="rId1" Type="t" Target="a.xml"/></Relationships>"#;
        assert_eq!(
            allowlisted(raw, &RELATIONSHIPS),
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships" xmlns:v="urn:vendor"></Relationships>"#
        );
    }

    #[test]
    fn an_allowlisted_part_is_a_fixed_point() {
        let raw = r#"<Types><Default Extension="xml" ContentType="x" Extra="1"/>text<x:y xmlns:x="urn:x"/></Types>"#;
        let once = allowlisted(raw, &CONTENT_TYPES);
        assert_eq!(allowlisted(&once, &CONTENT_TYPES), once);
    }

    #[test]
    fn a_foreign_root_is_refused() {
        assert!(
            PackageMarkup(&CONTENT_TYPES)
                .apply("<Relationships/>")
                .is_err()
        );
        let raw = r#"<Types xmlns="urn:vendor"/>"#;
        assert!(PackageMarkup(&CONTENT_TYPES).apply(raw).is_err());
    }

    /// The readers take only what the allowlist keeps: a foreign or nested
    /// entry and a qualified attribute are not read.
    #[test]
    fn the_readers_take_only_what_the_allowlist_keeps() {
        let manifest = r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types" xmlns:v="urn:vendor"><Default Extension="xml" ContentType="application/xml"/><v:Override PartName="/word/vendor.xml" ContentType="application/vnd.vendor"/><v:x><Override PartName="/word/nested.xml" ContentType="application/vnd.vendor"/></v:x><Override PartName="/word/document.xml" v:ContentType="application/vnd.vendor"/></Types>"#;
        let read = entries(manifest, &CONTENT_TYPES, "manifest").unwrap();
        assert_eq!(
            read,
            [
                (
                    "Default",
                    vec![
                        ("Extension", "xml".to_owned()),
                        ("ContentType", "application/xml".to_owned())
                    ]
                ),
                (
                    "Override",
                    vec![("PartName", "/word/document.xml".to_owned())]
                ),
            ]
        );
        let kept = allowlisted(manifest, &CONTENT_TYPES);
        assert_eq!(
            entries(&kept, &CONTENT_TYPES, "manifest").unwrap(),
            read,
            "{kept}"
        );
    }

    #[test]
    fn vocabularies_follow_the_part_path() {
        assert!(std::ptr::eq(
            vocabulary_of(&PartPath::from(CONTENT_TYPES_PART)).unwrap(),
            &CONTENT_TYPES
        ));
        assert!(std::ptr::eq(
            vocabulary_of(&PartPath::from("word/_rels/document.xml.rels")).unwrap(),
            &RELATIONSHIPS
        ));
        assert!(vocabulary_of(&PartPath::from("word/document.xml")).is_none());
    }
}
