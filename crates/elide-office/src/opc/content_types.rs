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

use crate::opc::package_xml::{CONTENT_TYPES, entries, without_entries};
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
    /// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if the manifest is not well-formed.
    pub(crate) fn parse(xml: &str) -> Result<Self> {
        let mut types = Self::default();
        for (name, attributes) in entries(xml, &CONTENT_TYPES, CONTENT_TYPES_PART)? {
            let value = |key: &str| {
                attributes
                    .iter()
                    .find(|(name, _)| *name == key)
                    .map(|(_, value)| value.clone())
            };
            let Some(content_type) = value("ContentType") else {
                continue;
            };
            match name {
                "Default" => {
                    if let Some(extension) = value("Extension") {
                        types
                            .defaults
                            .insert(extension.to_ascii_lowercase(), content_type);
                    }
                }
                _ => {
                    if let Some(part) = value("PartName") {
                        let path = part.trim_start_matches('/').to_ascii_lowercase();
                        types.overrides.insert(path, content_type);
                    }
                }
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
/// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if the manifest is not well-formed.
pub(crate) fn without_parts(xml: &str, removed: &HashSet<PartPath>) -> Result<String> {
    let removed: HashSet<String> = removed
        .iter()
        .map(|part| part.as_str().to_ascii_lowercase())
        .collect();
    without_entries(
        xml,
        &CONTENT_TYPES,
        CONTENT_TYPES_PART,
        "PartName",
        |name| removed.contains(name.to_ascii_lowercase().trim_start_matches('/')),
    )
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
