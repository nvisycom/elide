//! [`Relationship`]: one entry of an OPC relationships part, and the path
//! arithmetic that ties a relationships part to its source part and resolves a
//! relationship's target to a package part.
//!
//! Every part that references another (a document its header, a slide its
//! image) does so through a relationship in its sidecar `_rels/<name>.rels`
//! part; the package itself keeps its own in `_rels/.rels`. A relationship's
//! target is a URI relative to its source part's directory, or, with
//! `TargetMode="External"`, a URL outside the package. A relationships part
//! is read as [`package_xml`](super::package_xml) cuts it down to its OPC
//! vocabulary, so it reads the same before and after.

use std::collections::HashSet;
use std::ops::Range;

use elide_core::{Error, Result};

use crate::opc::ns::transitional_type;
use crate::opc::package_xml::{RELATIONSHIPS, SpannedEntry, spanned_entries, without_entries};
use crate::opc::part::PartPath;

/// The relationship type of an external hyperlink, the one external target that
/// is visible content (a link a reader follows) rather than a hidden reference.
pub(crate) const HYPERLINK: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink";

/// One relationship: its id, its type URI (a strict one as its transitional
/// twin), its raw target, and whether the target lies outside the package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Relationship {
    pub(crate) id: String,
    pub(crate) kind: String,
    pub(crate) target: String,
    pub(crate) external: bool,
}

impl Relationship {
    /// The package part this relationship's target names, resolved against the
    /// `source` part (`None` for the package's own relationships), or `None` for
    /// an external target.
    pub(crate) fn target_part(&self, source: Option<&PartPath>) -> Option<PartPath> {
        (!self.external).then(|| resolve(source, &self.target))
    }
}

/// Parse the relationships part `xml`.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if the
/// part is not well-formed, or a relationship has no `Id`.
pub(crate) fn parse(xml: &str) -> Result<Vec<Relationship>> {
    Ok(parse_spanned(xml)?
        .into_iter()
        .map(|(relationship, _)| relationship)
        .collect())
}

/// Parse the relationships part `xml`, each relationship with the range of
/// its `Target` value inside the quotes, if it has one.
///
/// # Errors
///
/// As [`parse`].
pub(crate) fn parse_spanned(xml: &str) -> Result<Vec<(Relationship, Option<Range<usize>>)>> {
    spanned_entries(xml, &RELATIONSHIPS, "relationships part")?
        .into_iter()
        .map(|SpannedEntry { entry, values, .. }| {
            let mut id = None;
            let mut kind = String::new();
            let mut target = String::new();
            let mut target_range = None;
            let mut external = false;
            for ((key, value), range) in entry.1.into_iter().zip(values) {
                match key {
                    "Id" => id = Some(value),
                    "Type" => kind = transitional_type(&value).into_owned(),
                    "Target" => {
                        target = value;
                        target_range = Some(range);
                    }
                    "TargetMode" => external = value == "External",
                    _ => {}
                }
            }
            // A relationship is removed by its id, so one without could not
            // be.
            let Some(id) = id else {
                return Err(Error::malformed(
                    "relationships part malformed: a relationship without an `Id`",
                ));
            };
            let relationship = Relationship {
                id,
                kind,
                target,
                external,
            };
            Ok((relationship, target_range))
        })
        .collect()
}

/// The relationships part `xml` with every relationship whose id is in
/// `removed` dropped, byte-for-byte otherwise.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if the part is not well-formed.
pub(crate) fn without(xml: &str, removed: &HashSet<String>) -> Result<String> {
    without_entries(xml, &RELATIONSHIPS, "relationships part", "Id", |id| {
        removed.contains(id)
    })
}

/// The source part of the relationships part `rels`: `word/document.xml` for
/// `word/_rels/document.xml.rels`, or `None` for the package's own
/// `_rels/.rels`.
pub(crate) fn source_of(rels: &PartPath) -> Option<PartPath> {
    let path = rels.as_str();
    let (dir, name) = path.rsplit_once("_rels/")?;
    let name = name.strip_suffix(".rels")?;
    (!name.is_empty()).then(|| PartPath::from(format!("{dir}{name}")))
}

/// The relationships part of `part`: `word/_rels/document.xml.rels` for
/// `word/document.xml`.
pub(crate) fn rels_of(part: &PartPath) -> PartPath {
    let path = part.as_str();
    match path.rsplit_once('/') {
        Some((dir, name)) => PartPath::from(format!("{dir}/_rels/{name}.rels")),
        None => PartPath::from(format!("_rels/{path}.rels")),
    }
}

/// Resolve a relationship `target` against its `source` part (the package root
/// when `None`) to a package-absolute part path.
///
/// An absolute target (`/word/media/x.png`) is package-rooted; a relative one is
/// joined onto the source part's directory. Backslashes are normalized to `/`
/// and `.` / `..` segments collapsed, so a target cannot climb out of the
/// package or name a part by a disguised path.
pub(crate) fn resolve(source: Option<&PartPath>, target: &str) -> PartPath {
    let target = target.replace('\\', "/");
    let joined = match (target.strip_prefix('/'), source) {
        (Some(absolute), _) => absolute.to_owned(),
        (None, Some(source)) => match source.as_str().rsplit_once('/') {
            Some((dir, _)) => format!("{dir}/{target}"),
            None => target,
        },
        (None, None) => target,
    };
    let mut segments: Vec<&str> = Vec::new();
    for segment in joined.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            other => segments.push(other),
        }
    }
    PartPath::from(segments.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const RELS: &str = r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="t/styles" Target="styles.xml"/><Relationship Id="rId2" Type="t/attachedTemplate" Target="file:///C:/Users/jsmith/Normal.dotm" TargetMode="External"/><Relationship Id="rId3" Type="t/image" Target="../media/image1.png"/></Relationships>"#;

    /// A qualified `Id` is not the relationship's id, in either order: the
    /// unqualified one decides whether the entry goes.
    #[test]
    fn a_qualified_id_does_not_shadow_the_id() {
        let entry = |attributes: &str| {
            format!(
                r#"<Relationship xmlns:v="urn:vendor" {attributes} Type="t/template" Target="https://example.com/private" TargetMode="External"/>"#
            )
        };
        let rels = |entry: &str| {
            format!(
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{entry}<Relationship Id="rId1" Type="t/styles" Target="styles.xml"/></Relationships>"#
            )
        };
        let removed: HashSet<String> = ["drop".to_owned()].into();
        for attributes in [r#"v:Id="keep" Id="drop""#, r#"Id="drop" v:Id="keep""#] {
            let raw = rels(&entry(attributes));
            assert_eq!(without(&raw, &removed).unwrap(), rels(""), "{attributes}");
        }
        // A kept id stays whatever its qualified shadow names.
        let raw = rels(&entry(r#"v:Id="drop" Id="keep""#));
        assert_eq!(without(&raw, &removed).unwrap(), raw);
    }

    /// A relationship without an unqualified `Id` is refused, a qualified
    /// one standing in for it or not.
    #[test]
    fn a_relationship_without_an_id_is_refused() {
        for attributes in ["", r#"xmlns:v="urn:vendor" v:Id="rId9""#] {
            let raw = format!(
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship {attributes} Type="t/template" Target="https://example.com/private" TargetMode="External"/></Relationships>"#
            );
            assert!(parse(&raw).is_err(), "{raw}");
        }
    }

    #[test]
    fn parses_ids_types_targets_and_modes() {
        let rels = parse(RELS).unwrap();
        assert_eq!(rels.len(), 3);
        assert!(rels[1].external);
        assert_eq!(rels[1].target_part(None), None);
        let source = PartPath::from("word/document.xml");
        assert_eq!(
            rels[2].target_part(Some(&source)),
            Some(PartPath::from("media/image1.png"))
        );
        assert_eq!(
            rels[0].target_part(Some(&source)),
            Some(PartPath::from("word/styles.xml"))
        );
    }

    #[test]
    fn without_drops_only_the_named_relationships() {
        let removed: HashSet<String> = ["rId2".to_owned()].into_iter().collect();
        let out = without(RELS, &removed).unwrap();
        assert!(!out.contains("jsmith"));
        assert!(out.contains(r#"Id="rId1""#) && out.contains(r#"Id="rId3""#));
    }

    #[test]
    fn a_removal_after_a_byte_order_mark_cuts_only_its_relationship() {
        let removed_rel =
            r#"<Relationship Id="rId2" Type="t/custom" Target="docProps/custom.xml"/>"#;
        let rels = format!(
            "\u{feff}<?xml version=\"1.0\"?><Relationships><Relationship Id=\"rId1\" Type=\"t/core\" Target=\"docProps/core.xml\" />{removed_rel}</Relationships>"
        );
        let removed: HashSet<String> = ["rId2".to_owned()].into_iter().collect();
        let out = without(&rels, &removed).unwrap();
        assert_eq!(out, rels.replace(removed_rel, ""));
    }

    #[test]
    fn source_and_rels_paths_are_inverse() {
        let part = PartPath::from("word/document.xml");
        let rels = rels_of(&part);
        assert_eq!(rels, PartPath::from("word/_rels/document.xml.rels"));
        assert_eq!(source_of(&rels), Some(part));
        assert_eq!(source_of(&PartPath::from("_rels/.rels")), None);
    }

    #[test]
    fn resolve_collapses_dot_segments_and_absolute_targets() {
        let source = PartPath::from("xl/workbook.xml");
        assert_eq!(
            resolve(Some(&source), "worksheets/./sheet1.xml"),
            PartPath::from("xl/worksheets/sheet1.xml")
        );
        assert_eq!(
            resolve(Some(&source), "/xl/styles.xml"),
            PartPath::from("xl/styles.xml")
        );
        assert_eq!(
            resolve(Some(&source), "..\\..\\..\\etc"),
            PartPath::from("etc")
        );
        assert_eq!(
            resolve(None, "word/document.xml"),
            PartPath::from("word/document.xml")
        );
    }
}
