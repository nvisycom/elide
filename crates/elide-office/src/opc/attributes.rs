//! [`TextAttribute`]: an attribute whose value is text a person wrote, which
//! element extraction (reading text nodes) never sees.
//!
//! A format names its text attributes in two closed lists. A visible one (a
//! sheet name, a dropdown entry) or one the policy keeps (a comment's author
//! under [`Redact`](elide_core::redaction::Disposition::Redact)) is surfaced
//! for detection like element text, through the format's
//! [`PartClassifier::text_attributes`](crate::opc::PartClassifier::text_attributes).
//! A hidden one (alternative text, a tooltip) is stripped by
//! [`StripAttributes`] and recorded.

use elide_core::redaction::HiddenContent;

use crate::opc::xml_edit::{Action, Attribute, Local, Name, Rules};

/// The namespace a [`TextAttribute`] part matches.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Ns {
    /// Any namespace, or none.
    Any,
    /// Exactly this namespace.
    In(&'static str),
    /// No namespace: an unqualified attribute.
    None,
}

impl Ns {
    fn matches(self, namespace: Option<&str>) -> bool {
        match self {
            Self::Any => true,
            Self::In(expected) => namespace == Some(expected),
            Self::None => namespace.is_none(),
        }
    }
}

/// An attribute that carries text a person wrote: the element it sits on and
/// its own name, each matched by namespace and local name.
#[derive(Debug, Clone, Copy)]
pub struct TextAttribute {
    pub(crate) element: (Ns, Local),
    pub(crate) attribute: (Ns, &'static str),
}

impl TextAttribute {
    /// The attribute `attribute` on the element `element`.
    pub(crate) const fn new(element: (Ns, Local), attribute: (Ns, &'static str)) -> Self {
        Self { element, attribute }
    }

    /// Whether `attribute` on `element` is this one.
    pub(crate) fn matches(&self, element: Name<'_>, attribute: Name<'_>) -> bool {
        let (element_ns, element_local) = self.element;
        let (attribute_ns, attribute_local) = self.attribute;
        element_ns.matches(element.namespace)
            && element_local.matches(element.local)
            && attribute_ns.matches(attribute.namespace)
            && attribute.local == attribute_local
    }
}

/// Element rules that remove the attributes `attributes` from every element,
/// all of them the one item of hidden content `content`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct StripAttributes {
    pub(crate) content: HiddenContent,
    pub(crate) attributes: &'static [TextAttribute],
}

impl Rules for StripAttributes {
    fn content(&self) -> Option<HiddenContent> {
        Some(self.content)
    }

    fn element(&self, _name: Name<'_>, _attributes: &[Attribute<'_>]) -> Action {
        Action::Keep
    }

    fn removes_attribute(&self, element: Name<'_>, attribute: &Attribute<'_>) -> bool {
        self.attributes
            .iter()
            .any(|text| text.matches(element, attribute.name))
    }
}

/// Element rules that give the attributes `attributes` the neutral `value`, for
/// attributes a format requires and so cannot remove, all of them the one item
/// of hidden content `content`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct NeutralizeAttributes {
    pub(crate) content: HiddenContent,
    pub(crate) attributes: &'static [TextAttribute],
    pub(crate) value: &'static str,
}

impl Rules for NeutralizeAttributes {
    fn content(&self) -> Option<HiddenContent> {
        Some(self.content)
    }

    fn element(&self, _name: Name<'_>, _attributes: &[Attribute<'_>]) -> Action {
        Action::Keep
    }

    fn removes_attribute(&self, _element: Name<'_>, _attribute: &Attribute<'_>) -> bool {
        false
    }

    fn neutralizes_attribute(
        &self,
        element: Name<'_>,
        attribute: &Attribute<'_>,
    ) -> Option<&'static str> {
        self.attributes
            .iter()
            .any(|text| text.matches(element, attribute.name))
            .then_some(self.value)
    }
}

/// The neutral timestamp a required date is given: the same for every date, so
/// it says nothing about when.
pub(crate) const NEUTRAL_DATE: &str = "1970-01-01T00:00:00Z";

/// Alternative text, in every format: the description and title of a drawing
/// or picture, and a legacy VML shape's alternative text and title, which may
/// hold the file name the picture was inserted from.
pub(crate) const ALTERNATIVE_TEXT: StripAttributes = StripAttributes {
    content: HiddenContent::AlternativeText,
    attributes: &[
        TextAttribute::new((Ns::Any, Local::Exact("docPr")), (Ns::None, "descr")),
        TextAttribute::new((Ns::Any, Local::Exact("docPr")), (Ns::None, "title")),
        TextAttribute::new((Ns::Any, Local::Exact("cNvPr")), (Ns::None, "descr")),
        TextAttribute::new((Ns::Any, Local::Exact("cNvPr")), (Ns::None, "title")),
        TextAttribute::new((Ns::In(VML), Local::Any), (Ns::None, "alt")),
        TextAttribute::new((Ns::In(VML), Local::Any), (Ns::None, "title")),
        TextAttribute::new(
            (Ns::In(VML), Local::Exact("imagedata")),
            (Ns::In(OFFICE), "title"),
        ),
    ],
};

/// Tooltips, in every format: the text a hyperlink shows on hover.
pub(crate) const TOOLTIPS: StripAttributes = StripAttributes {
    content: HiddenContent::AlternativeText,
    attributes: &[
        TextAttribute::new((Ns::Any, Local::Exact("hyperlink")), (Ns::Any, "tooltip")),
        TextAttribute::new((Ns::Any, Local::Exact("hlinkClick")), (Ns::None, "tooltip")),
        TextAttribute::new((Ns::Any, Local::Exact("hlinkHover")), (Ns::None, "tooltip")),
    ],
};

/// The legacy VML namespace.
const VML: &str = "urn:schemas-microsoft-com:vml";
/// The legacy Office drawing namespace.
const OFFICE: &str = "urn:schemas-microsoft-com:office:office";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opc::xml_edit::edit;

    const DESCRIPTION: &[TextAttribute] = &[TextAttribute::new(
        (Ns::Any, Local::Exact("cNvPr")),
        (Ns::None, "descr"),
    )];

    #[test]
    fn strips_a_named_attribute_from_any_namespace() {
        let raw =
            r#"<p:cNvPr xmlns:p="urn:p" id="2" name="Picture 1" descr="C:\Users\jane\photo.jpg"/>"#;
        let rules = StripAttributes {
            content: HiddenContent::AlternativeText,
            attributes: DESCRIPTION,
        };
        let edited = edit(raw, &rules).unwrap().unwrap();
        assert_eq!(edited.count, 1);
        assert_eq!(
            edited.text,
            r#"<p:cNvPr xmlns:p="urn:p" id="2" name="Picture 1"/>"#
        );
    }

    #[test]
    fn a_neutralized_attribute_edits_once() {
        const CREATED: &[TextAttribute] = &[TextAttribute::new(
            (Ns::Any, Local::Exact("cm")),
            (Ns::None, "created"),
        )];
        let rules = NeutralizeAttributes {
            content: HiddenContent::Comment,
            attributes: CREATED,
            value: NEUTRAL_DATE,
        };
        let raw = r#"<cm id="1" created="2024-05-06T07:08:09.123"/>"#;
        let edited = edit(raw, &rules).unwrap().unwrap();
        assert_eq!(
            edited.text,
            format!(r#"<cm id="1" created="{NEUTRAL_DATE}"/>"#)
        );
        assert!(edit(&edited.text, &rules).unwrap().is_none());
    }
}
