//! [`ContentTypes`]: the package's `[Content_Types].xml` manifest, which names
//! the content type of every part.
//!
//! OPC types a part by an `<Override PartName="/word/document.xml" …>` for that
//! exact part, else by a `<Default Extension="xml" …>` for its extension. Part
//! names and extensions compare ASCII case-insensitively, as OPC requires.

use std::collections::{HashMap, HashSet};

use elide_core::{Error, Result};
use quick_xml::Reader;
use quick_xml::escape::unescape;
use quick_xml::events::Event;

use crate::opc::part::PartPath;

/// The manifest's own part path.
pub(crate) const CONTENT_TYPES_PART: &str = "[Content_Types].xml";

/// The parsed content-types manifest.
#[derive(Debug, Clone, Default)]
pub(crate) struct ContentTypes {
    /// Lowercased extension → content type.
    defaults: HashMap<String, String>,
    /// Lowercased part path (no leading `/`) → content type.
    overrides: HashMap<String, String>,
}

impl ContentTypes {
    /// Parse the manifest `xml`.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::MalformedInput`] if the manifest is not well-formed.
    pub(crate) fn parse(xml: &str) -> Result<Self> {
        let mut types = Self::default();
        let mut reader = Reader::from_str(xml);
        loop {
            let element = match reader
                .read_event()
                .map_err(|e| Error::malformed(format!("[Content_Types].xml malformed: {e}")))?
            {
                Event::Eof => break,
                Event::Empty(e) | Event::Start(e) => e,
                _ => continue,
            };
            let mut key = None;
            let mut content_type = None;
            for attr in element.attributes() {
                let attr = attr
                    .map_err(|e| Error::malformed(format!("[Content_Types].xml malformed: {e}")))?;
                let value = unescape(&attr.value)
                    .map_err(|e| Error::malformed(format!("[Content_Types].xml malformed: {e}")))?;
                match attr.key.local_name().as_ref() {
                    "Extension" | "PartName" => key = Some(value.to_ascii_lowercase()),
                    "ContentType" => content_type = Some(value.into_owned()),
                    _ => {}
                }
            }
            let (Some(key), Some(content_type)) = (key, content_type) else {
                continue;
            };
            match element.local_name().as_ref() {
                "Default" => {
                    types.defaults.insert(key, content_type);
                }
                "Override" => {
                    let path = key.trim_start_matches('/').to_owned();
                    types.overrides.insert(path, content_type);
                }
                _ => {}
            }
        }
        Ok(types)
    }

    /// The content type of `part`: its override, else its extension's default,
    /// else `None`.
    pub(crate) fn of(&self, part: &PartPath) -> Option<&str> {
        self.overrides
            .get(&part.as_str().to_ascii_lowercase())
            .or_else(|| {
                part.extension()
                    .and_then(|ext| self.defaults.get(&ext.to_ascii_lowercase()))
            })
            .map(String::as_str)
    }
}

/// The manifest `xml` with the overrides of every part in `removed` dropped,
/// byte-for-byte otherwise.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`] if the manifest is not well-formed.
pub(crate) fn without_parts(xml: &str, removed: &HashSet<PartPath>) -> Result<String> {
    let removed: HashSet<String> = removed
        .iter()
        .map(|part| part.as_str().to_ascii_lowercase())
        .collect();
    let mut reader = Reader::from_str(xml);
    let mut out = String::with_capacity(xml.len());
    let mut cursor = 0;
    let mut last = 0;
    loop {
        let event = reader
            .read_event()
            .map_err(|e| Error::malformed(format!("[Content_Types].xml malformed: {e}")))?;
        let span = last..reader.buffer_position() as usize;
        last = span.end;
        let element = match event {
            Event::Eof => break,
            Event::Empty(e) if e.local_name().as_ref() == "Override" => e,
            _ => continue,
        };
        let part_name = element
            .attributes()
            .flatten()
            .find(|attr| attr.key.local_name().as_ref() == "PartName")
            .and_then(|attr| unescape(&attr.value).ok().map(|v| v.to_ascii_lowercase()));
        if part_name.is_some_and(|name| removed.contains(name.trim_start_matches('/'))) {
            out.push_str(&xml[cursor..span.start]);
            cursor = span.end;
        }
    }
    out.push_str(&xml[cursor..]);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Default Extension="PNG" ContentType="image/png"/><Override PartName="/word/document.xml" ContentType="main"/><Override PartName="/word/comments.xml" ContentType="comments"/></Types>"#;

    #[test]
    fn an_override_wins_over_the_extension_default() {
        let types = ContentTypes::parse(MANIFEST).unwrap();
        assert_eq!(types.of(&PartPath::from("word/document.xml")), Some("main"));
        assert_eq!(
            types.of(&PartPath::from("customXml/item1.xml")),
            Some("application/xml")
        );
        assert_eq!(
            types.of(&PartPath::from("word/media/image1.png")),
            Some("image/png")
        );
        assert_eq!(types.of(&PartPath::from("word/media/image1.emf")), None);
    }

    #[test]
    fn part_names_compare_case_insensitively() {
        let types = ContentTypes::parse(MANIFEST).unwrap();
        assert_eq!(types.of(&PartPath::from("Word/Document.xml")), Some("main"));
    }

    #[test]
    fn without_parts_drops_only_the_removed_overrides() {
        let removed: HashSet<PartPath> =
            [PartPath::from("word/comments.xml")].into_iter().collect();
        let out = without_parts(MANIFEST, &removed).unwrap();
        assert!(!out.contains("comments"));
        assert!(out.contains(r#"PartName="/word/document.xml""#));
        assert_eq!(
            out.len(),
            MANIFEST.len()
                - r#"<Override PartName="/word/comments.xml" ContentType="comments"/>"#.len()
        );
    }
}
