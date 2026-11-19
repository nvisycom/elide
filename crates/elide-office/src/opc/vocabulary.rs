//! The XML vocabularies a document-property part may use: every element and
//! attribute of a property part the codec ships must be in one of these
//! namespaces.
//!
//! Every other XML part the codec ships is rebuilt from its schema, which
//! keeps only the namespaces the schema names, so the check of an encoded
//! package needs this list only for the property parts, the one kind of XML
//! part kept but not rebuilt. The list is closed on purpose: content in a
//! namespace this crate has not reviewed may hold text that extraction never
//! reads, so a part using one is refused rather than shipped.

use std::cell::RefCell;

use elide_core::redaction::HiddenContent;

use crate::opc::ns::{
    CORE_PROPERTIES, CUSTOM_PROPERTIES, DC, DCMITYPE, DCTERMS, DOC_PROPS_VTYPES,
    EXTENDED_PROPERTIES, XML, XMLNS, XSI,
};
use crate::opc::schema;
use crate::opc::xml_edit::{Action, Attribute, Name, Rules};

/// The reviewed namespaces: XML itself, the core properties with the Dublin
/// Core they use, and the extended and custom properties with their variant
/// types. A strict namespace is read as its transitional twin.
const VOCABULARIES: &[&str] = &[
    XML,
    XMLNS,
    XSI,
    CORE_PROPERTIES,
    DC,
    DCTERMS,
    DCMITYPE,
    EXTENDED_PROPERTIES,
    CUSTOM_PROPERTIES,
    DOC_PROPS_VTYPES,
];

/// Whether `namespace` is a reviewed vocabulary.
fn is_known(namespace: &str) -> bool {
    VOCABULARIES.contains(&schema::transitional(namespace))
}

/// Element rules that change nothing and record the first element or
/// attribute in an unreviewed namespace.
#[derive(Default)]
pub(crate) struct Vocabulary {
    /// The first unreviewed namespace met.
    unknown: RefCell<Option<String>>,
}

impl Vocabulary {
    /// The first unreviewed namespace met, if any.
    pub(crate) fn unknown(&self) -> Option<String> {
        self.unknown.borrow().clone()
    }

    /// Record `namespace` as unreviewed, unless one already is.
    fn record(&self, namespace: &str) {
        self.unknown
            .borrow_mut()
            .get_or_insert_with(|| namespace.to_owned());
    }
}

impl Rules for Vocabulary {
    fn content(&self) -> Option<HiddenContent> {
        None
    }

    fn element(&self, name: Name<'_>, attributes: &[Attribute<'_>]) -> Action {
        // Every property element is namespaced; one in no namespace (or under
        // an undeclared prefix) is in no vocabulary at all.
        match name.namespace {
            Some(namespace) if is_known(namespace) => {}
            Some(namespace) => self.record(namespace),
            None => self.record("(no namespace)"),
        }
        // Unqualified attributes are the norm; a qualified one must be known.
        for attribute in attributes {
            if let Some(namespace) = attribute.name.namespace
                && !is_known(namespace)
            {
                self.record(namespace);
            }
        }
        Action::Keep
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opc::xml_edit::edit;

    #[test]
    fn records_the_first_unreviewed_namespace() {
        let vocabulary = Vocabulary::default();
        let raw = r#"<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:v="urn:vendor:secret"><v:note>hidden</v:note></cp:coreProperties>"#;
        edit(raw, &vocabulary).unwrap();
        assert_eq!(vocabulary.unknown().as_deref(), Some("urn:vendor:secret"));
    }

    #[test]
    fn an_element_in_no_namespace_is_unreviewed() {
        let vocabulary = Vocabulary::default();
        edit(
            "<cp:coreProperties><cp:revision/></cp:coreProperties>",
            &vocabulary,
        )
        .unwrap();
        assert_eq!(vocabulary.unknown().as_deref(), Some("(no namespace)"));
    }

    #[test]
    fn a_story_vocabulary_is_unreviewed_here() {
        let vocabulary = Vocabulary::default();
        let raw =
            r#"<w:p xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"/>"#;
        edit(raw, &vocabulary).unwrap();
        assert_eq!(
            vocabulary.unknown().as_deref(),
            Some("http://schemas.openxmlformats.org/wordprocessingml/2006/main")
        );
    }

    #[test]
    fn a_reviewed_part_passes() {
        let vocabulary = Vocabulary::default();
        let raw = r#"<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"><dc:creator xml:lang="en">x</dc:creator><dcterms:created xsi:type="dcterms:W3CDTF">2024-01-01T00:00:00Z</dcterms:created></cp:coreProperties>"#;
        edit(raw, &vocabulary).unwrap();
        assert_eq!(vocabulary.unknown(), None);
    }

    #[test]
    fn a_strict_property_part_passes() {
        let vocabulary = Vocabulary::default();
        let raw = r#"<Properties xmlns="http://purl.oclc.org/ooxml/officeDocument/extendedProperties" xmlns:vt="http://purl.oclc.org/ooxml/officeDocument/docPropsVTypes"><TitlesOfParts><vt:vector size="1" baseType="lpstr"><vt:lpstr>x</vt:lpstr></vt:vector></TitlesOfParts></Properties>"#;
        edit(raw, &vocabulary).unwrap();
        assert_eq!(vocabulary.unknown(), None);
    }
}
