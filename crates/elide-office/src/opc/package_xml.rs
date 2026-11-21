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
use quick_xml::events::{BytesStart, Event};

use crate::opc::content_types::CONTENT_TYPES_PART;
use crate::opc::part::PartPath;
use crate::opc::xml_edit::{
    Cut, Edit, Edited, Skipped, SpannedReader, attribute_extent, attribute_source, cut_events,
    refuse_doctype, splice,
};
use crate::opc::{attribute_value, malformed_xml, ns};

/// The vocabulary of a package part: its root, and each entry the root may
/// hold with the unqualified attributes it may carry.
#[derive(Debug)]
pub(crate) struct Vocabulary {
    /// What the part is called in an error.
    part: &'static str,
    /// The namespace of the root and its entries.
    namespace: &'static str,
    /// The root's local name.
    root: &'static str,
    /// Each entry the root may hold.
    entries: &'static [EntryRule],
}

/// An entry a [`Vocabulary`]'s root may hold.
#[derive(Debug)]
pub(crate) struct EntryRule {
    /// The entry's local name.
    name: &'static str,
    /// The unqualified attributes it may carry.
    attributes: &'static [&'static str],
}

/// The vocabulary of `[Content_Types].xml`.
pub(crate) static CONTENT_TYPES: Vocabulary = Vocabulary {
    part: CONTENT_TYPES_PART,
    namespace: ns::CONTENT_TYPES,
    root: "Types",
    entries: &[
        EntryRule {
            name: "Default",
            attributes: &["Extension", "ContentType"],
        },
        EntryRule {
            name: "Override",
            attributes: &["PartName", "ContentType"],
        },
    ],
};

/// The vocabulary of a relationships part.
pub(crate) static RELATIONSHIPS: Vocabulary = Vocabulary {
    part: "relationships part",
    namespace: ns::RELATIONSHIPS,
    root: "Relationships",
    entries: &[EntryRule {
        name: "Relationship",
        attributes: &["Id", "Type", "Target", "TargetMode"],
    }],
};

/// The vocabulary of the package part `path`, or `None` when it is neither
/// the manifest nor a relationships part.
pub(crate) fn vocabulary_of(path: &PartPath) -> Option<&'static Vocabulary> {
    if path.is_content_types() {
        Some(&CONTENT_TYPES)
    } else if path.is_relationships() {
        Some(&RELATIONSHIPS)
    } else {
        None
    }
}

/// An entry the package part's readers take: its local name, where it is
/// written, and its allowed attributes.
#[derive(Debug)]
pub(crate) struct Entry {
    /// The entry's local name.
    pub(crate) name: &'static str,
    /// The span of its whole element.
    pub(crate) span: Range<usize>,
    /// Its allowed attributes, in the order they are written.
    pub(crate) attributes: Vec<EntryAttribute>,
}

/// An allowed attribute of an [`Entry`].
#[derive(Debug)]
pub(crate) struct EntryAttribute {
    /// The attribute's local name.
    pub(crate) key: &'static str,
    /// Its decoded value.
    pub(crate) value: String,
    /// The range of its value inside the quotes.
    pub(crate) range: Range<usize>,
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
    attribute: &str,
    drops: impl Fn(&str) -> bool,
) -> Result<String> {
    let cuts = entries(xml, vocabulary)?
        .into_iter()
        .filter(|entry| {
            entry
                .attributes
                .iter()
                .any(|attr| attr.key == attribute && drops(&attr.value))
        })
        .map(|entry| Cut::remove(entry.span))
        .collect::<Vec<Cut<&str>>>();
    Ok(splice(xml, cuts))
}

/// The entries of the package part `xml` as [`PackageMarkup`] keeps them:
/// the root's children in `vocabulary`'s namespace (or none), with their
/// unqualified allowed attributes. Anything the allowlist cuts is not read,
/// so the part reads the same before and after it.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if
/// `xml` is not well-formed, naming the part as `vocabulary` calls it.
pub(crate) fn entries(xml: &str, vocabulary: &Vocabulary) -> Result<Vec<Entry>> {
    let malformed =
        |e: &dyn std::fmt::Display| Error::malformed(format!("{} malformed: {e}", vocabulary.part));
    let mut reader = SpannedReader::new_ns(xml);
    let mut entries = Vec::new();
    // An entry whose element is still open, read to its end tag.
    let mut open: Option<(Entry, Skipped)> = None;
    let mut depth = 0usize;
    loop {
        let (namespace, event, span) = reader.read_resolved_event().map_err(|e| malformed(&e))?;
        if let Some((_, skipped)) = &mut open {
            if let Some(whole) = skipped.read(&event, span.end)? {
                let (mut entry, _) = open.take().expect("an open entry");
                entry.span = whole;
                entries.push(entry);
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
            && let Some(rule) = vocabulary.entry(namespace.as_deref(), e.local_name().into_inner())
        {
            let mut attributes = Vec::new();
            for attr in e.attributes() {
                let attr = attr.map_err(|e| malformed(&e))?;
                if attr.key.prefix().is_some() {
                    continue;
                }
                let Some(key) = rule
                    .attributes
                    .iter()
                    .find(|key| **key == attr.key.local_name().into_inner())
                else {
                    continue;
                };
                let value = attribute_value(&attr.value).map_err(|e| malformed(&e))?;
                let range = attribute_source(xml, &attr)?.value;
                attributes.push(EntryAttribute {
                    key,
                    value: value.into_owned(),
                    range,
                });
            }
            let entry = Entry {
                name: rule.name,
                span: span.clone(),
                attributes,
            };
            if is_start {
                open = Some((entry, Skipped::new(span.start)));
                depth += 1;
            } else {
                entries.push(entry);
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

    /// The entry `local` in `namespace` with the attributes it may carry, or
    /// `None` when the root holds no such entry.
    fn entry(&self, namespace: Option<&str>, local: &str) -> Option<&'static EntryRule> {
        if !self.owns(namespace) {
            return None;
        }
        self.entries.iter().find(|rule| rule.name == local)
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
        let mut cuts: Vec<Cut<&str>> = Vec::new();
        let mut open: Vec<Kept> = Vec::new();
        let mut removing: Option<Skipped> = None;
        let mut seen_root = false;
        loop {
            let (namespace, event, span) = reader.read_resolved_event().map_err(malformed_xml)?;
            refuse_doctype(&event)?;
            if Skipped::skip(&mut removing, &event, span.end, &mut cuts)? {
                continue;
            }
            match event {
                // The part is its root element, whole.
                Event::Eof if !seen_root => {
                    return Err(malformed_xml("a part without a root element"));
                }
                Event::Eof if !open.is_empty() => return Err(malformed_xml("unclosed element")),
                Event::Eof => break,
                Event::PI(_) => cuts.push(Cut::remove(span)),
                Event::Start(ref e) | Event::Empty(ref e) => {
                    let is_start = matches!(event, Event::Start(_));
                    let local = e.local_name().into_inner();
                    let kept = match open.last() {
                        None if seen_root => {
                            return Err(malformed_xml("a second root element"));
                        }
                        None if vocabulary.owns(namespace.as_deref())
                            && local == vocabulary.root =>
                        {
                            seen_root = true;
                            Kept::Root
                        }
                        None => {
                            return Err(malformed_xml(format_args!(
                                "the root element is not `{}`",
                                vocabulary.root
                            )));
                        }
                        Some(Kept::Root) => match vocabulary.entry(namespace.as_deref(), local) {
                            Some(rule) => Kept::Entry(rule.attributes),
                            None => {
                                removing = Skipped::begin(span, is_start, &mut cuts);
                                continue;
                            }
                        },
                        // An entry holds nothing.
                        Some(Kept::Entry(_)) => {
                            removing = Skipped::begin(span, is_start, &mut cuts);
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
        let count = cuts.len();
        Ok(Edited::from_cuts(raw, cuts, count))
    }
}

/// Cut every attribute of `e` but a namespace declaration or an unqualified
/// one named in `allowed`, each with its leading whitespace.
fn cut_attributes(
    raw: &str,
    e: &BytesStart<'_>,
    allowed: &[&str],
    cuts: &mut Vec<Cut<&str>>,
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
        cuts.push(Cut::remove(attribute_extent(raw, &attr)?.whole));
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
        cut_events(raw, |event| match event {
            Event::Text(text) => !text.bytes().all(|byte| byte.is_ascii_whitespace()),
            Event::CData(_) | Event::GeneralRef(_) => true,
            _ => false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="main"/></Types>"#;

    const RELS: &str = r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="t/styles" Target="styles.xml"/><Relationship Id="rId2" Type="t/link" Target="https://example.com" TargetMode="External"/></Relationships>"#;

    /// The entries of the manifest `xml`, each as its name and its attributes'
    /// keys and values.
    fn read(xml: &str) -> Vec<(&'static str, Vec<(&'static str, String)>)> {
        entries(xml, &CONTENT_TYPES)
            .unwrap()
            .into_iter()
            .map(|entry| {
                let attributes = entry
                    .attributes
                    .into_iter()
                    .map(|attr| (attr.key, attr.value))
                    .collect();
                (entry.name, attributes)
            })
            .collect()
    }

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

    /// A truncated part, one without a root, and one with a second root are
    /// refused rather than cut down.
    #[test]
    fn a_part_that_is_not_one_whole_root_is_refused() {
        let truncated = r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/>"#;
        assert!(PackageMarkup(&CONTENT_TYPES).apply(truncated).is_err());
        let open_entry = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="t" Target="a.xml">"#;
        assert!(PackageMarkup(&RELATIONSHIPS).apply(open_entry).is_err());
        assert!(
            PackageMarkup(&CONTENT_TYPES)
                .apply("<?xml version=\"1.0\"?>")
                .is_err()
        );
        let twice = r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"/><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"/>"#;
        assert!(PackageMarkup(&CONTENT_TYPES).apply(twice).is_err());
    }

    /// A document type declaration is refused before the root and inside an
    /// entry being cut alike.
    #[test]
    fn a_document_type_is_refused() {
        let declared = r#"<!DOCTYPE Types [<!ENTITY x "Jane">]><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"/>"#;
        assert!(PackageMarkup(&CONTENT_TYPES).apply(declared).is_err());
        let nested = r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Vendor><!DOCTYPE d></Vendor></Types>"#;
        assert!(PackageMarkup(&CONTENT_TYPES).apply(nested).is_err());
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
        let manifest_entries = read(manifest);
        assert_eq!(
            manifest_entries,
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
        assert_eq!(read(&kept), manifest_entries, "{kept}");
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
