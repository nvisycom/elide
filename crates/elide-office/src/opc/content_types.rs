//! [`ContentTypes`]: the package's `[Content_Types].xml` manifest, which names
//! the content type of every part.
//!
//! OPC types a part by an `<Override PartName="/word/document.xml" …>` for that
//! exact part, else by a `<Default Extension="xml" …>` for its extension. Part
//! names and extensions compare ASCII case-insensitively, as OPC requires.
//! The manifest is read as [`package_xml`](super::package_xml) cuts it down
//! to its OPC vocabulary, so it types every part the same before and after.

use std::collections::{HashMap, HashSet};

use elide_core::Result;

use crate::opc::ct;
use crate::opc::package_xml::{CONTENT_TYPES, Entry, EntryAttribute, entries, without_entries};
use crate::opc::part::PartPath;

/// The manifest's own part path.
pub(crate) const CONTENT_TYPES_PART: &str = "[Content_Types].xml";

/// A part's content type, lowercased, as every content type compares ASCII
/// case-insensitively.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ContentType(String);

impl ContentType {
    /// The content type written `raw`.
    pub(crate) fn new(raw: &str) -> Self {
        Self(raw.to_ascii_lowercase())
    }

    /// The content type, lowercased.
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether a part of this type holds XML: an XML type, or the legacy VML
    /// drawing type (XML that predates the `+xml` naming).
    pub(crate) fn is_xml(&self) -> bool {
        self.0.ends_with("xml") || self.is_vml_drawing()
    }

    /// Whether this is the legacy VML drawing type.
    pub(crate) fn is_vml_drawing(&self) -> bool {
        self.0 == ct::VML_DRAWING
    }
}

/// The parsed content-types manifest.
#[derive(Debug, Clone, Default)]
pub(crate) struct ContentTypes {
    /// Lowercased extension → content type.
    defaults: HashMap<String, ContentType>,
    /// Lowercased part path (no leading `/`) → content type.
    overrides: HashMap<String, ContentType>,
}

impl ContentTypes {
    /// Parse the manifest `xml`.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if the manifest is not well-formed.
    pub(crate) fn parse(xml: &str) -> Result<Self> {
        let mut types = Self::default();
        for Entry {
            name, attributes, ..
        } in entries(xml, &CONTENT_TYPES)?
        {
            let mut content_type = None;
            let mut extension = None;
            let mut part_name = None;
            for EntryAttribute { key, value, .. } in attributes {
                let slot = match key {
                    "ContentType" => &mut content_type,
                    "Extension" => &mut extension,
                    "PartName" => &mut part_name,
                    _ => continue,
                };
                slot.get_or_insert(value);
            }
            let Some(content_type) = content_type else {
                continue;
            };
            let content_type = ContentType::new(&content_type);
            match name {
                "Default" => {
                    if let Some(extension) = extension {
                        types
                            .defaults
                            .insert(extension.to_ascii_lowercase(), content_type);
                    }
                }
                "Override" => {
                    if let Some(part) = part_name {
                        let path = part.trim_start_matches('/').to_ascii_lowercase();
                        types.overrides.insert(path, content_type);
                    }
                }
                _ => {}
            }
        }
        Ok(types)
    }

    /// The content type of `part`: its override, else its extension's default,
    /// else `None`.
    pub(crate) fn of(&self, part: &PartPath) -> Option<&ContentType> {
        self.overrides
            .get(&part.as_str().to_ascii_lowercase())
            .or_else(|| {
                part.extension()
                    .and_then(|ext| self.defaults.get(&ext.to_ascii_lowercase()))
            })
    }
}

/// The manifest `xml` with the overrides of every part in `removed` dropped,
/// byte-for-byte otherwise.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if the manifest is not well-formed.
pub(crate) fn without_parts(xml: &str, removed: &HashSet<PartPath>) -> Result<String> {
    let removed: HashSet<String> = removed
        .iter()
        .map(|part| part.as_str().to_ascii_lowercase())
        .collect();
    without_entries(xml, &CONTENT_TYPES, "PartName", |name| {
        removed.contains(name.to_ascii_lowercase().trim_start_matches('/'))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Default Extension="PNG" ContentType="image/png"/><Override PartName="/word/document.xml" ContentType="main"/><Override PartName="/word/comments.xml" ContentType="comments"/></Types>"#;

    #[test]
    fn a_removal_after_a_byte_order_mark_cuts_only_its_override() {
        let removed_override =
            r#"<Override PartName="/docProps/custom.xml" ContentType="custom"/>"#;
        let manifest = format!(
            "\u{feff}<?xml version=\"1.0\"?><Types><Override PartName=\"/word/document.xml\" ContentType=\"main\" />{removed_override}</Types>"
        );
        let removed: HashSet<PartPath> = [PartPath::from("docProps/custom.xml")].into();
        let out = without_parts(&manifest, &removed).unwrap();
        assert_eq!(out, manifest.replace(removed_override, ""));
    }

    /// The content type `types` names for the part at `path`.
    fn type_of<'a>(types: &'a ContentTypes, path: &str) -> Option<&'a str> {
        types.of(&PartPath::from(path)).map(ContentType::as_str)
    }

    #[test]
    fn an_override_wins_over_the_extension_default() {
        let types = ContentTypes::parse(MANIFEST).unwrap();
        assert_eq!(type_of(&types, "word/document.xml"), Some("main"));
        assert_eq!(
            type_of(&types, "customXml/item1.xml"),
            Some("application/xml")
        );
        assert_eq!(type_of(&types, "word/media/image1.png"), Some("image/png"));
        assert_eq!(type_of(&types, "word/media/image1.emf"), None);
    }

    #[test]
    fn part_names_compare_case_insensitively() {
        let types = ContentTypes::parse(MANIFEST).unwrap();
        assert_eq!(type_of(&types, "Word/Document.xml"), Some("main"));
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
