//! The XML vocabularies an OOXML part may use: every element and attribute of a
//! part the codec ships must be in one of these namespaces.
//!
//! The list is closed on purpose. Office writes vendor and future extensions in
//! namespaces of their own (and marks them ignorable, so an older reader skips
//! them); content in a namespace this crate has not reviewed may hold text that
//! extraction never reads. A part using one is refused rather than shipped,
//! and a namespace is added here only once its content is known to be either
//! extracted or free of user data.

use std::cell::RefCell;

use elide_core::redaction::HiddenContent;

use crate::opc::xml_edit::{Action, Attribute, Name, Rules};

/// The reviewed namespaces: OPC packaging, the ECMA-376 transitional and strict
/// vocabularies, Dublin Core for the core properties, legacy VML and Office,
/// and the Microsoft extensions Office writes by default.
const VOCABULARIES: &[&str] = &[
    // XML itself.
    "http://www.w3.org/XML/1998/namespace",
    "http://www.w3.org/2000/xmlns/",
    "http://www.w3.org/2001/XMLSchema-instance",
    // OPC packaging and core properties.
    "http://schemas.openxmlformats.org/package/2006/content-types",
    "http://schemas.openxmlformats.org/package/2006/relationships",
    "http://schemas.openxmlformats.org/package/2006/metadata/core-properties",
    "http://purl.org/dc/elements/1.1/",
    "http://purl.org/dc/terms/",
    "http://purl.org/dc/dcmitype/",
    "http://schemas.openxmlformats.org/markup-compatibility/2006",
    // ECMA-376 transitional.
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships",
    "http://schemas.openxmlformats.org/officeDocument/2006/math",
    "http://schemas.openxmlformats.org/officeDocument/2006/sharedTypes",
    "http://schemas.openxmlformats.org/officeDocument/2006/extended-properties",
    "http://schemas.openxmlformats.org/officeDocument/2006/custom-properties",
    "http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes",
    "http://schemas.openxmlformats.org/officeDocument/2006/bibliography",
    "http://schemas.openxmlformats.org/officeDocument/2006/customXml",
    "http://schemas.openxmlformats.org/wordprocessingml/2006/main",
    "http://schemas.openxmlformats.org/presentationml/2006/main",
    "http://schemas.openxmlformats.org/spreadsheetml/2006/main",
    "http://schemas.openxmlformats.org/drawingml/2006/main",
    "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing",
    "http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing",
    "http://schemas.openxmlformats.org/drawingml/2006/picture",
    "http://schemas.openxmlformats.org/drawingml/2006/chart",
    "http://schemas.openxmlformats.org/drawingml/2006/chartDrawing",
    "http://schemas.openxmlformats.org/drawingml/2006/diagram",
    "http://schemas.openxmlformats.org/drawingml/2006/lockedCanvas",
    "http://schemas.openxmlformats.org/drawingml/2006/compatibility",
    "http://schemas.openxmlformats.org/schemaLibrary/2006/main",
    // ECMA-376 strict.
    "http://purl.oclc.org/ooxml/officeDocument/relationships",
    "http://purl.oclc.org/ooxml/officeDocument/math",
    "http://purl.oclc.org/ooxml/officeDocument/sharedTypes",
    "http://purl.oclc.org/ooxml/officeDocument/extendedProperties",
    "http://purl.oclc.org/ooxml/officeDocument/customProperties",
    "http://purl.oclc.org/ooxml/officeDocument/docPropsVTypes",
    "http://purl.oclc.org/ooxml/wordprocessingml/main",
    "http://purl.oclc.org/ooxml/presentationml/main",
    "http://purl.oclc.org/ooxml/spreadsheetml/main",
    "http://purl.oclc.org/ooxml/drawingml/main",
    "http://purl.oclc.org/ooxml/drawingml/wordprocessingDrawing",
    "http://purl.oclc.org/ooxml/drawingml/spreadsheetDrawing",
    "http://purl.oclc.org/ooxml/drawingml/picture",
    "http://purl.oclc.org/ooxml/drawingml/chart",
    "http://purl.oclc.org/ooxml/drawingml/chartDrawing",
    "http://purl.oclc.org/ooxml/drawingml/diagram",
    // Legacy VML and Office drawing.
    "urn:schemas-microsoft-com:vml",
    "urn:schemas-microsoft-com:office:office",
    "urn:schemas-microsoft-com:office:word",
    "urn:schemas-microsoft-com:office:excel",
    "urn:schemas-microsoft-com:office:powerpoint",
    // Word extensions.
    "http://schemas.microsoft.com/office/word/2006/wordml",
    "http://schemas.microsoft.com/office/word/2010/wordml",
    "http://schemas.microsoft.com/office/word/2010/wordprocessingDrawing",
    "http://schemas.microsoft.com/office/word/2010/wordprocessingShape",
    "http://schemas.microsoft.com/office/word/2010/wordprocessingGroup",
    "http://schemas.microsoft.com/office/word/2010/wordprocessingCanvas",
    "http://schemas.microsoft.com/office/word/2010/wordprocessingInk",
    "http://schemas.microsoft.com/office/word/2012/wordml",
    "http://schemas.microsoft.com/office/word/2015/wordml/symex",
    "http://schemas.microsoft.com/office/word/2016/wordml/cid",
    "http://schemas.microsoft.com/office/word/2018/wordml",
    "http://schemas.microsoft.com/office/word/2018/wordml/cex",
    "http://schemas.microsoft.com/office/word/2020/wordml/sdtdatahash",
    "http://schemas.microsoft.com/office/word/2023/wordml/word16du",
    "http://schemas.microsoft.com/office/word/2024/wordml/sdtformatlock",
    // PowerPoint extensions.
    "http://schemas.microsoft.com/office/powerpoint/2010/main",
    "http://schemas.microsoft.com/office/powerpoint/2012/main",
    "http://schemas.microsoft.com/office/powerpoint/2017/10/main",
    "http://schemas.microsoft.com/office/powerpoint/2018/8/main",
    // A modern comment's anchor: the slide, shape, or text range it is on, by
    // id only.
    "http://schemas.microsoft.com/office/powerpoint/2013/main/command",
    "http://schemas.microsoft.com/office/drawing/2013/main/command",
    // Excel extensions.
    "http://schemas.microsoft.com/office/excel/2006/main",
    "http://schemas.microsoft.com/office/spreadsheetml/2009/9/main",
    "http://schemas.microsoft.com/office/spreadsheetml/2009/9/ac",
    "http://schemas.microsoft.com/office/spreadsheetml/2010/11/main",
    "http://schemas.microsoft.com/office/spreadsheetml/2010/11/ac",
    "http://schemas.microsoft.com/office/spreadsheetml/2014/revision",
    "http://schemas.microsoft.com/office/spreadsheetml/2015/revision2",
    "http://schemas.microsoft.com/office/spreadsheetml/2016/revision3",
    "http://schemas.microsoft.com/office/spreadsheetml/2016/revision6",
    "http://schemas.microsoft.com/office/spreadsheetml/2016/revision10",
    "http://schemas.microsoft.com/office/spreadsheetml/2015/02/main",
    "http://schemas.microsoft.com/office/spreadsheetml/2018/calcfeatures",
    // Threaded comments and the people who wrote them: the comment text is
    // element text, the people's names text attributes.
    "http://schemas.microsoft.com/office/spreadsheetml/2018/threadedcomments",
    // DrawingML and chart extensions.
    "http://schemas.microsoft.com/office/drawing/2007/8/2/chart",
    "http://schemas.microsoft.com/office/drawing/2008/diagram",
    "http://schemas.microsoft.com/office/drawing/2010/main",
    "http://schemas.microsoft.com/office/drawing/2010/chart",
    "http://schemas.microsoft.com/office/drawing/2010/slicer",
    "http://schemas.microsoft.com/office/drawing/2012/chart",
    "http://schemas.microsoft.com/office/drawing/2012/chartStyle",
    "http://schemas.microsoft.com/office/drawing/2014/main",
    "http://schemas.microsoft.com/office/drawing/2014/chart",
    "http://schemas.microsoft.com/office/drawing/2014/chartex",
    "http://schemas.microsoft.com/office/drawing/2015/9/8/chartex",
    "http://schemas.microsoft.com/office/drawing/2015/10/21/chartex",
    "http://schemas.microsoft.com/office/drawing/2016/5/9/chartex",
    "http://schemas.microsoft.com/office/drawing/2016/5/10/chartex",
    "http://schemas.microsoft.com/office/drawing/2016/5/11/chartex",
    "http://schemas.microsoft.com/office/drawing/2016/5/12/chartex",
    "http://schemas.microsoft.com/office/drawing/2016/5/13/chartex",
    "http://schemas.microsoft.com/office/drawing/2016/5/14/chartex",
    "http://schemas.microsoft.com/office/drawing/2016/ink",
    "http://schemas.microsoft.com/office/drawing/2016/SVG/main",
    "http://schemas.microsoft.com/office/drawing/2016/11/main",
    "http://schemas.microsoft.com/office/drawing/2017/decorative",
    "http://schemas.microsoft.com/office/drawing/2017/model3d",
    "http://schemas.microsoft.com/office/2019/extlst",
    "http://schemas.microsoft.com/office/thememl/2012/main",
];

/// Whether `namespace` is a reviewed vocabulary.
pub(crate) fn is_known(namespace: &str) -> bool {
    VOCABULARIES.contains(&namespace)
}

/// Element rules that change nothing and record the first element or
/// attribute in an unreviewed namespace.
#[derive(Default)]
pub(crate) struct Vocabulary {
    unknown: RefCell<Option<String>>,
    /// Whether an element in no namespace is allowed.
    unqualified: bool,
}

impl Vocabulary {
    /// The rules for a legacy VML drawing, which unlike the rest of OOXML has a
    /// root (`<xml>`) and HTML text markup (`<div>`, `<font>`) in no namespace.
    /// Its text is read like any other drawing's.
    pub(crate) fn vml() -> Self {
        Self {
            unqualified: true,
            ..Self::default()
        }
    }

    /// The first unreviewed namespace met, if any.
    pub(crate) fn unknown(&self) -> Option<String> {
        self.unknown.borrow().clone()
    }

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
        // Every other OOXML element is namespaced; one in no namespace (or under
        // an undeclared prefix) is in no vocabulary at all.
        match name.namespace {
            Some(namespace) if is_known(namespace) => {}
            Some(namespace) => self.record(namespace),
            None if self.unqualified => {}
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

    fn removes_attribute(&self, _element: Name<'_>, _attribute: &Attribute<'_>) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opc::xml_edit::edit;

    #[test]
    fn records_the_first_unreviewed_namespace() {
        let vocabulary = Vocabulary::default();
        let raw = r#"<w:p xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:v="urn:vendor:secret"><v:note>hidden</v:note></w:p>"#;
        edit(raw, &vocabulary).unwrap();
        assert_eq!(vocabulary.unknown().as_deref(), Some("urn:vendor:secret"));
    }

    #[test]
    fn an_element_in_no_namespace_is_unreviewed() {
        let vocabulary = Vocabulary::default();
        edit("<w:p><w:r/></w:p>", &vocabulary).unwrap();
        assert_eq!(vocabulary.unknown().as_deref(), Some("(no namespace)"));
    }

    #[test]
    fn a_vml_drawing_may_use_no_namespace_but_no_unreviewed_one() {
        let vocabulary = Vocabulary::vml();
        let raw = r#"<xml xmlns:v="urn:schemas-microsoft-com:vml"><v:shape><v:textbox><div>x</div></v:textbox></v:shape></xml>"#;
        edit(raw, &vocabulary).unwrap();
        assert_eq!(vocabulary.unknown(), None);

        let vocabulary = Vocabulary::vml();
        edit(
            r#"<xml xmlns:s="urn:vendor:secret"><s:note/></xml>"#,
            &vocabulary,
        )
        .unwrap();
        assert_eq!(vocabulary.unknown().as_deref(), Some("urn:vendor:secret"));
    }

    #[test]
    fn a_reviewed_part_passes() {
        let vocabulary = Vocabulary::default();
        let raw = r#"<w:p xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml"><w:r><w:t xml:space="preserve">x</w:t></w:r></w:p>"#;
        edit(raw, &vocabulary).unwrap();
        assert_eq!(vocabulary.unknown(), None);
    }
}
