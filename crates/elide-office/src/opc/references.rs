//! [`References`]: element rules over the attributes (`r:id`, `r:embed`,
//! `r:link`, …, and VML's `o:relid`) by which an XML part names one of its
//! relationships, in the transitional relationships namespace or its strict
//! twin.

use std::cell::RefCell;
use std::collections::HashSet;

use elide_core::redaction::HiddenContent;
use quick_xml::escape::unescape;

use crate::opc::ns::{R, VML_OFFICE};
use crate::opc::schema;
use crate::opc::xml_edit::{Action, Attribute, Name, Rules};

/// Element rules over the relationship-naming attributes (`r:id`, `r:embed`,
/// `r:link`, …): drop every element naming a removed relationship by an
/// attribute the schemas declare, and any other such attribute alone, or
/// record the first name that resolves to no relationship.
pub(crate) struct References<'a> {
    /// Under `drop`: the ids to remove. Under `check`: the ids that exist.
    ids: Ids<'a>,
    /// The first name met that resolves to no relationship.
    dangling: RefCell<Option<String>>,
}

/// The id set [`References`] matches against.
enum Ids<'a> {
    /// The ids whose elements go.
    Drop(&'a HashSet<String>),
    /// The ids that exist.
    Present(&'a HashSet<&'a str>),
}

impl<'a> References<'a> {
    /// Rules dropping every element that names one of `ids`.
    pub(crate) fn dropping(ids: &'a HashSet<String>) -> Self {
        Self {
            ids: Ids::Drop(ids),
            dangling: RefCell::new(None),
        }
    }

    /// Rules recording the first name that is not one of `ids`.
    pub(crate) fn checking(ids: &'a HashSet<&'a str>) -> Self {
        Self {
            ids: Ids::Present(ids),
            dangling: RefCell::new(None),
        }
    }

    /// The first name met that resolves to no relationship, if any.
    pub(crate) fn dangling(&self) -> Option<String> {
        self.dangling.borrow().clone()
    }
}

impl Rules for References<'_> {
    /// An element naming a dropped relationship goes with it, counted once with
    /// the relationship rather than again per element.
    fn content(&self) -> Option<HiddenContent> {
        None
    }

    fn element(&self, name: Name<'_>, attributes: &[Attribute<'_>]) -> Action {
        for attribute in attributes.iter().filter(|attribute| naming(attribute)) {
            match &self.ids {
                // Only an attribute the element's type declares takes the
                // element along; the rebuild cuts any other, so it goes alone.
                Ids::Drop(ids) => {
                    if schema::declares_reference(name, attribute.name) && dropped(ids, attribute) {
                        return Action::Remove;
                    }
                }
                Ids::Present(ids) => {
                    let id = unescape(attribute.value).ok();
                    if !id.as_deref().is_some_and(|id| ids.contains(id)) {
                        self.dangling.borrow_mut().get_or_insert_with(|| {
                            id.map_or_else(|| attribute.value.to_owned(), |id| id.into_owned())
                        });
                    }
                }
            }
        }
        Action::Keep
    }

    fn removes_attribute(&self, _element: Name<'_>, attribute: &Attribute<'_>) -> bool {
        matches!(&self.ids, Ids::Drop(ids) if naming(attribute) && dropped(ids, attribute))
    }
}

/// Whether `attribute` names a relationship: any in the relationships
/// namespace, and VML's `o:relid`.
fn naming(attribute: &Attribute<'_>) -> bool {
    attribute.name.namespace == Some(R) || is_relid(attribute)
}

/// Whether `attribute` is VML's `o:relid`, by which a VML shape's fill,
/// stroke or image names its relationship.
fn is_relid(attribute: &Attribute<'_>) -> bool {
    attribute.name.namespace == Some(VML_OFFICE) && attribute.name.local == "relid"
}

/// Whether `attribute` names one of the dropped `ids`. The id compares as the
/// value XML reads, its references decoded; one that does not decode names no
/// relationship, so it goes under `drop` and dangles under `check`.
fn dropped(ids: &HashSet<String>, attribute: &Attribute<'_>) -> bool {
    unescape(attribute.value).map_or(true, |id| ids.contains(id.as_ref()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opc::xml_edit::{edit, expect_edit};

    const STRICT: &str = r#"<w:document xmlns:w="http://purl.oclc.org/ooxml/wordprocessingml/main" xmlns:r="http://purl.oclc.org/ooxml/officeDocument/relationships"><w:body><w:p><w:hyperlink r:id="rId1"/><w:hyperlink r:id="rId2"/></w:p></w:body></w:document>"#;

    const VML_RELID: &str = r#"<xml xmlns:v="urn:schemas-microsoft-com:vml" xmlns:o="urn:schemas-microsoft-com:office:office"><v:shape><v:imagedata o:relid="rId1" o:title="kept"/><v:fill o:relid="rId2"/></v:shape></xml>"#;

    /// VML's `o:relid` names a relationship as `r:id` does.
    #[test]
    fn a_vml_relid_names_a_relationship() {
        let dropped: HashSet<String> = ["rId1".to_owned()].into_iter().collect();
        let edited = expect_edit(VML_RELID, &References::dropping(&dropped));
        assert!(!edited.text.contains("imagedata"), "{}", edited.text);
        assert!(
            edited.text.contains(r#"<v:fill o:relid="rId2"/>"#),
            "{}",
            edited.text
        );

        let present: HashSet<&str> = ["rId2"].into_iter().collect();
        let references = References::checking(&present);
        edit(VML_RELID, &references).unwrap();
        assert_eq!(references.dangling().as_deref(), Some("rId1"));
    }

    #[test]
    fn a_strict_reference_to_a_missing_relationship_dangles() {
        let ids: HashSet<&str> = ["rId1"].into_iter().collect();
        let references = References::checking(&ids);
        edit(STRICT, &references).unwrap();
        assert_eq!(references.dangling().as_deref(), Some("rId2"));
    }

    #[test]
    fn a_strict_reference_to_a_dropped_relationship_goes() {
        let ids: HashSet<String> = ["rId2".to_owned()].into_iter().collect();
        let edited = expect_edit(STRICT, &References::dropping(&ids));
        assert!(edited.text.contains(r#"r:id="rId1""#));
        assert!(!edited.text.contains("rId2"));
    }

    /// An id spelled with a character reference names the same relationship.
    #[test]
    fn an_escaped_reference_compares_decoded() {
        let raw = r#"<w:p xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><w:hyperlink r:id="rId&#50;"/><w:hyperlink r:id="rId1"/></w:p>"#;
        let present: HashSet<&str> = ["rId1", "rId2"].into_iter().collect();
        let references = References::checking(&present);
        edit(raw, &references).unwrap();
        assert_eq!(references.dangling(), None);

        let dropped: HashSet<String> = ["rId2".to_owned()].into_iter().collect();
        let edited = expect_edit(raw, &References::dropping(&dropped));
        assert!(!edited.text.contains("rId&#50;"));
        assert!(edited.text.contains(r#"r:id="rId1""#));
    }

    /// A relationship attribute its element's type does not declare goes
    /// alone, though another type declares it; a declared one takes its
    /// element along.
    #[test]
    fn a_reference_its_element_does_not_declare_goes_alone() {
        let raw = r#"<w:body xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><w:p r:id="rId2"><w:r><w:t>kept</w:t></w:r></w:p><w:hyperlink r:id="rId2"><w:r><w:t>link</w:t></w:r></w:hyperlink><a:blip r:embed="rId2"/></w:body>"#;
        let dropped: HashSet<String> = ["rId2".to_owned()].into_iter().collect();
        let edited = expect_edit(raw, &References::dropping(&dropped));
        assert!(
            edited.text.contains("<w:p><w:r><w:t>kept"),
            "{}",
            edited.text
        );
        for gone in ["rId2", "link", "a:blip"] {
            assert!(!edited.text.contains(gone), "{gone} in {}", edited.text);
        }
    }

    /// An attribute no schema declares in the relationships namespace goes
    /// alone when it names a dropped relationship, its element kept.
    #[test]
    fn an_undeclared_reference_goes_without_its_element() {
        let raw = r#"<w:p xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" r:unused="rId2"><w:r><w:t>kept</w:t></w:r></w:p>"#;
        let dropped: HashSet<String> = ["rId2".to_owned()].into_iter().collect();
        let edited = expect_edit(raw, &References::dropping(&dropped));
        assert!(edited.text.contains("kept"), "{}", edited.text);
        assert!(!edited.text.contains("rId2"), "{}", edited.text);

        // It still dangles under `check`.
        let present = HashSet::new();
        let references = References::checking(&present);
        edit(raw, &references).unwrap();
        assert_eq!(references.dangling().as_deref(), Some("rId2"));
    }

    #[test]
    fn a_transitional_reference_still_dangles() {
        let raw = r#"<w:p xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><w:hyperlink r:id="rId7"/></w:p>"#;
        let ids = HashSet::new();
        let references = References::checking(&ids);
        edit(raw, &references).unwrap();
        assert_eq!(references.dangling().as_deref(), Some("rId7"));
    }
}
