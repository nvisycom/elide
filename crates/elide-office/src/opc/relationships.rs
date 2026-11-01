//! [`Relationship`]: one entry of an OPC relationships part, and the path
//! arithmetic that ties a relationships part to its source part and resolves a
//! relationship's target to a package part.
//!
//! Every part that references another (a document its header, a slide its
//! image) does so through a relationship in its sidecar `_rels/<name>.rels`
//! part; the package itself keeps its own in `_rels/.rels`. A relationship's
//! target is a URI relative to its source part's directory, or, with
//! `TargetMode="External"`, a URL outside the package.

use std::collections::HashSet;

use elide_core::{Error, Result};
use quick_xml::Reader;
use quick_xml::escape::unescape;
use quick_xml::events::Event;

use crate::opc::part::PartPath;

/// The relationship type of an external hyperlink, the one external target that
/// is visible content (a link a reader follows) rather than a hidden reference.
pub(crate) const HYPERLINK: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink";

/// The namespace of the attributes (`r:id`, `r:embed`, `r:link`, …) by which an
/// XML part names one of its relationships.
pub(crate) const RELATIONSHIP_ATTRIBUTES: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

/// One relationship: its id, its type URI, its raw target, and whether the
/// target lies outside the package.
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
/// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if the part is not well-formed.
pub(crate) fn parse(xml: &str) -> Result<Vec<Relationship>> {
    let mut reader = Reader::from_str(xml);
    let mut relationships = Vec::new();
    loop {
        let element = match reader
            .read_event()
            .map_err(|e| Error::malformed(format!("relationships part malformed: {e}")))?
        {
            Event::Eof => break,
            Event::Empty(e) | Event::Start(e) if e.local_name().as_ref() == "Relationship" => e,
            _ => continue,
        };
        let mut relationship = Relationship {
            id: String::new(),
            kind: String::new(),
            target: String::new(),
            external: false,
        };
        for attr in element.attributes() {
            let attr =
                attr.map_err(|e| Error::malformed(format!("relationships part malformed: {e}")))?;
            let value = unescape(&attr.value)
                .map_err(|e| Error::malformed(format!("relationships part malformed: {e}")))?;
            match attr.key.local_name().as_ref() {
                "Id" => relationship.id = value.into_owned(),
                "Type" => relationship.kind = value.into_owned(),
                "Target" => relationship.target = value.into_owned(),
                "TargetMode" => relationship.external = value == "External",
                _ => {}
            }
        }
        relationships.push(relationship);
    }
    Ok(relationships)
}

/// The relationships part `xml` with every relationship whose id is in
/// `removed` dropped, byte-for-byte otherwise.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if the part is not well-formed.
pub(crate) fn without(xml: &str, removed: &HashSet<String>) -> Result<String> {
    let mut reader = Reader::from_str(xml);
    let mut out = String::with_capacity(xml.len());
    let mut cursor = 0;
    let mut last = 0;
    loop {
        let event = reader
            .read_event()
            .map_err(|e| Error::malformed(format!("relationships part malformed: {e}")))?;
        let span = last..reader.buffer_position() as usize;
        last = span.end;
        let element = match event {
            Event::Eof => break,
            Event::Empty(e) if e.local_name().as_ref() == "Relationship" => e,
            _ => continue,
        };
        let id = element
            .attributes()
            .flatten()
            .find(|attr| attr.key.local_name().as_ref() == "Id")
            .and_then(|attr| unescape(&attr.value).ok().map(|v| v.into_owned()));
        if id.is_some_and(|id| removed.contains(&id)) {
            out.push_str(&xml[cursor..span.start]);
            cursor = span.end;
        }
    }
    out.push_str(&xml[cursor..]);
    Ok(out)
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
