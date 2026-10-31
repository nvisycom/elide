//! [`References`]: element rules over the attributes (`r:id`, `r:embed`,
//! `r:link`, …) by which an XML part names one of its relationships.

use std::cell::RefCell;
use std::collections::HashSet;

use crate::opc::relationships::RELATIONSHIP_ATTRIBUTES;
use crate::opc::xml_edit::{Action, Attribute, Name, Rules};

/// Element rules over the relationship-naming attributes (`r:id`, `r:embed`,
/// `r:link`, …): drop every element naming a removed relationship, or record
/// the first name that resolves to no relationship.
pub(crate) struct References<'a> {
    /// Under `drop`: the ids to remove. Under `check`: the ids that exist.
    ids: Ids<'a>,
    dangling: RefCell<Option<String>>,
}

/// The id set [`References`] matches against.
enum Ids<'a> {
    Drop(&'a HashSet<String>),
    Present(&'a HashSet<&'a str>),
}

impl<'a> References<'a> {
    pub(crate) fn dropping(ids: &'a HashSet<String>) -> Self {
        Self {
            ids: Ids::Drop(ids),
            dangling: RefCell::new(None),
        }
    }

    pub(crate) fn checking(ids: &'a HashSet<&'a str>) -> Self {
        Self {
            ids: Ids::Present(ids),
            dangling: RefCell::new(None),
        }
    }

    pub(crate) fn dangling(&self) -> Option<String> {
        self.dangling.borrow().clone()
    }
}

impl Rules for References<'_> {
    fn element(&self, _name: Name<'_>, attributes: &[Attribute<'_>]) -> Action {
        let named = attributes
            .iter()
            .filter(|attribute| attribute.name.in_namespace(RELATIONSHIP_ATTRIBUTES));
        for attribute in named {
            match self.ids {
                Ids::Drop(ids) if ids.contains(attribute.value) => return Action::Remove,
                Ids::Present(ids) if !ids.contains(attribute.value) => {
                    self.dangling
                        .borrow_mut()
                        .get_or_insert_with(|| attribute.value.to_owned());
                }
                _ => {}
            }
        }
        Action::Keep
    }

    fn removes_attribute(&self, _element: Name<'_>, _attribute: &Attribute<'_>) -> bool {
        false
    }
}
