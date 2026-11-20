//! [`References`]: element rules over the attributes (`r:id`, `r:embed`,
//! `r:link`, …) by which an XML part names one of its relationships, in the
//! transitional relationships namespace or its strict twin.

use std::cell::RefCell;
use std::collections::HashSet;

use elide_core::redaction::HiddenContent;
use quick_xml::escape::unescape;

use crate::opc::ns::R;
use crate::opc::schema;
use crate::opc::xml_edit::{Action, Attribute, Name, Rules};

/// Element rules over the relationship-naming attributes (`r:id`, `r:embed`,
/// `r:link`, …): drop every element naming a removed relationship, or record
/// the first name that resolves to no relationship.
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

    fn element(&self, _name: Name<'_>, attributes: &[Attribute<'_>]) -> Action {
        // A strict part names its relationships in the strict twin of `r:`.
        let named = attributes.iter().filter(|attribute| {
            attribute
                .name
                .namespace
                .is_some_and(|namespace| schema::transitional(namespace) == R)
        });
        for attribute in named {
            // The id compares as the value XML reads, its references decoded.
            // One that does not decode names no relationship: it goes under
            // `drop`, and dangles under `check`.
            let id = unescape(attribute.value).ok();
            match (&self.ids, id.as_deref()) {
                (Ids::Drop(_), None) => return Action::Remove,
                (Ids::Drop(ids), Some(id)) if ids.contains(id) => return Action::Remove,
                (Ids::Present(ids), id) if !id.is_some_and(|id| ids.contains(id)) => {
                    self.dangling
                        .borrow_mut()
                        .get_or_insert_with(|| id.unwrap_or(attribute.value).to_owned());
                }
                _ => {}
            }
        }
        Action::Keep
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opc::xml_edit::{edit, expect_edit};

    const STRICT: &str = r#"<w:document xmlns:w="http://purl.oclc.org/ooxml/wordprocessingml/main" xmlns:r="http://purl.oclc.org/ooxml/officeDocument/relationships"><w:body><w:p><w:hyperlink r:id="rId1"/><w:hyperlink r:id="rId2"/></w:p></w:body></w:document>"#;

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

    #[test]
    fn a_transitional_reference_still_dangles() {
        let raw = r#"<w:p xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><w:hyperlink r:id="rId7"/></w:p>"#;
        let ids = HashSet::new();
        let references = References::checking(&ids);
        edit(raw, &references).unwrap();
        assert_eq!(references.dangling().as_deref(), Some("rId7"));
    }
}
