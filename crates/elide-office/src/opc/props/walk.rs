//! The walk every reading of a property part shares: [`Walk`] follows the
//! open elements, matching each by its local name in its own vocabulary, and
//! says what field an element holds ([`Entered`], in its [`Place`]).
//! [`fields`](super::fields), [`rewrite`](super::rewrite), and
//! [`UnsurfacedProperties`](super::strip::UnsurfacedProperties) all read a
//! part by it, so what is stripped and what is read never disagree.

use std::borrow::Cow;
use std::collections::HashMap;

use elide_core::entity::{LabelRef, builtins};
use quick_xml::events::BytesStart;

use super::{CUSTOM_PROPERTY, ELEMENT_FIELDS, LISTS, TEXT_TYPES, TIMESTAMPS};
use crate::opc::attribute_value;
use crate::opc::props::vocabulary::is_known;
use crate::opc::xml::edit::local_name;
use crate::opc::xml::ns::{
    CORE_PROPERTIES, CUSTOM_PROPERTIES, DC, DCTERMS, DOC_PROPS_VTYPES, EXTENDED_PROPERTIES,
    transitional,
};

/// Where a field's value sits in its part.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Place {
    /// A core or extended property element, its value its text.
    Element,
    /// A text entry of a list, keyed `<list>/<index>` (`TitlesOfParts/2`).
    Entry,
    /// A custom property, keyed by its position, its value in one typed
    /// child.
    Custom,
}

impl Place {
    /// What a field in this place, held by the element `local`, is: an
    /// element field by the schema's meaning for it, any other field free
    /// content.
    fn label(self, local: &str) -> LabelRef {
        let label = match (self, local) {
            (Self::Element, "creator" | "lastModifiedBy" | "Manager") => &builtins::PERSON_NAME,
            (Self::Element, "created" | "modified") => &builtins::DATE_TIME,
            (Self::Element, "Company") => &builtins::ORGANIZATION_NAME,
            _ => &builtins::COMMUNICATIONS_CONTENT,
        };
        LabelRef::from(&**label)
    }
}

/// A field an element holds: its key in the part, where its value sits, what
/// it is, whether it holds a typed timestamp, and for a custom property, its
/// name.
pub(super) struct Entered {
    pub(super) key: String,
    pub(super) place: Place,
    pub(super) label: LabelRef,
    pub(super) timestamp: bool,
    pub(super) name: Option<String>,
}

impl Entered {
    /// The field `key` in `place`, held by the element `local`.
    fn new(key: String, place: Place, local: &str) -> Self {
        Self {
            key,
            place,
            label: place.label(local),
            timestamp: place == Place::Element && TIMESTAMPS.contains(&local),
            name: None,
        }
    }

    /// The key of the field holding a custom property's name.
    pub(super) fn name_key(&self) -> String {
        format!("{}/name", self.key)
    }
}

/// An open element of a property part, as a [`Walk`] matches it.
#[derive(Debug)]
enum Open {
    /// An element in an unreviewed namespace, or inside one.
    Foreign,
    /// An element out of the vocabulary its local name belongs to, which
    /// matches no name.
    Misplaced,
    /// An element in its vocabulary, by its local name.
    Known(String),
}

impl Open {
    /// Whether this is the element `local`.
    fn is(&self, local: &str) -> bool {
        matches!(self, Self::Known(known) if known == local)
    }

    /// Whether this is a list of the document's parts.
    fn is_list(&self) -> bool {
        self.list().is_some()
    }

    /// The list of the document's parts this is, if it is one.
    fn list(&self) -> Option<&str> {
        match self {
            Self::Known(local) if LISTS.contains(&local.as_str()) => Some(local),
            _ => None,
        }
    }
}

/// The open elements of a property part, which say what field an element is.
#[derive(Debug, Default)]
pub(super) struct Walk {
    /// The open elements, the root first.
    open: Vec<Open>,
    /// How many text entries of the current list have been entered.
    entries: usize,
    /// How many named custom properties have been entered.
    properties: usize,
    /// How many times each element field has been entered.
    seen: HashMap<String, usize>,
    /// Where the outermost open element in an unreviewed namespace sits:
    /// nothing in it is a field or read.
    outside: Option<usize>,
}

/// The namespace an element this walk reads by name must be in to be that
/// element, a strict one read as its transitional twin.
fn vocabulary(local: &str) -> Option<&'static str> {
    match local {
        "creator" | "title" | "subject" | "description" => Some(DC),
        "lastModifiedBy" | "keywords" | "category" => Some(CORE_PROPERTIES),
        "created" | "modified" => Some(DCTERMS),
        "Manager" | "Company" | "Template" | "HyperlinkBase" | "TitlesOfParts" | "HeadingPairs" => {
            Some(EXTENDED_PROPERTIES)
        }
        CUSTOM_PROPERTY => Some(CUSTOM_PROPERTIES),
        "vector" | "variant" | "array" | "lpwstr" | "lpstr" | "bstr" | "i4" => {
            Some(DOC_PROPS_VTYPES)
        }
        _ => None,
    }
}

impl Walk {
    /// Enter `elem`, in `namespace`, returning the field it holds, if any.
    ///
    /// An element is the one its local name says only in that name's
    /// vocabulary; in another namespace it is no field, and one in an
    /// unreviewed namespace, or in none, holds nothing read.
    ///
    /// Every key is unique in its part. An element field is keyed by its name,
    /// a repeated one with `#<n>` after it (no element name holds a `#`). A
    /// list entry is keyed `<list>/<n>`, and a custom property by its position,
    /// `property/<n>`, since its name is free text that could look like any
    /// key; the name is a field of its own, `property/<n>/name`.
    pub(super) fn enter(
        &mut self,
        namespace: Option<&str>,
        elem: &BytesStart<'_>,
    ) -> Option<Entered> {
        let namespace = namespace.map(transitional);
        if self.outside.is_some() || namespace.is_none_or(|namespace| !is_known(namespace)) {
            self.outside.get_or_insert(self.open.len());
            self.open.push(Open::Foreign);
            return None;
        }
        let local = local_name(elem);
        let (entered, open) = match vocabulary(&local) {
            Some(expected) if namespace != Some(expected) => (None, Open::Misplaced),
            _ => (self.field(&local, elem), Open::Known(local)),
        };
        self.open.push(open);
        entered
    }

    /// The field the element `local` (`elem`), opening in its vocabulary,
    /// holds, if any.
    fn field(&mut self, local: &str, elem: &BytesStart<'_>) -> Option<Entered> {
        if self.open.len() == 1 {
            return self.property(local, elem);
        }
        // A list's entries sit in its vector, a heading's in a variant.
        let in_list = matches!(self.open.len(), 3 | 4) && self.is_list_path();
        if !in_list || !TEXT_TYPES.contains(&local) {
            return None;
        }
        let key = format!("{}/{}", self.list()?, self.entries);
        self.entries += 1;
        Some(Entered::new(key, Place::Entry, local))
    }

    /// The field the element `local` (`elem`), opening as a child of the
    /// root, holds, if any: an element field or a named custom property. A
    /// list starts its entries.
    fn property(&mut self, local: &str, elem: &BytesStart<'_>) -> Option<Entered> {
        if ELEMENT_FIELDS.contains(&local) {
            let count = self.seen.entry(local.to_owned()).or_default();
            *count += 1;
            let key = match *count {
                1 => local.to_owned(),
                n => format!("{local}#{n}"),
            };
            return Some(Entered::new(key, Place::Element, local));
        }
        if local == CUSTOM_PROPERTY {
            let name = elem
                .try_get_attribute("name")
                .ok()
                .flatten()
                .and_then(|name| attribute_value(&name.value).ok().map(Cow::into_owned))?;
            let key = format!("{CUSTOM_PROPERTY}/{}", self.properties);
            self.properties += 1;
            return Some(Entered {
                name: Some(name),
                ..Entered::new(key, Place::Custom, local)
            });
        }
        if LISTS.contains(&local) {
            self.entries = 0;
        }
        None
    }

    /// Leave the innermost open element.
    pub(super) fn leave(&mut self) {
        self.open.pop();
        if self.outside == Some(self.open.len()) {
            self.outside = None;
        }
    }

    /// Whether an open element is in an unreviewed namespace.
    pub(super) fn is_foreign(&self) -> bool {
        self.outside.is_some()
    }

    /// How many elements are open.
    pub(super) fn depth(&self) -> usize {
        self.open.len()
    }

    /// The list of the document's parts an open element is, if any.
    pub(super) fn list(&self) -> Option<&str> {
        match self.open.as_slice() {
            [_, list, ..] => list.list(),
            _ => None,
        }
    }

    /// Whether the innermost open element is a custom property.
    pub(super) fn is_property(&self) -> bool {
        matches!(self.open.as_slice(), [_, property] if property.is(CUSTOM_PROPERTY))
    }

    /// Whether the innermost open element is a heading's count: an `i4`
    /// variant of the heading pairs' vector.
    pub(super) fn is_count(&self) -> bool {
        matches!(
            self.open.as_slice(),
            [_, pairs, vector, variant, count]
                if pairs.is("HeadingPairs")
                    && vector.is("vector")
                    && variant.is("variant")
                    && count.is("i4")
        )
    }

    /// Whether the innermost open element is a list container, its vector,
    /// or a variant of that vector: the path to a list entry.
    pub(super) fn is_list_path(&self) -> bool {
        match self.open.as_slice() {
            [_, list] => list.is_list(),
            [_, list, vector] => list.is_list() && vector.is("vector"),
            [_, list, vector, variant] => {
                list.is_list() && vector.is("vector") && variant.is("variant")
            }
            _ => false,
        }
    }
}
