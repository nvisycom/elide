//! The XML namespaces named in more than one place, each by the prefix Office
//! writes it under where it has a well-known one. The transitional URIs are
//! listed; a strict part's namespaces map onto these through [`transitional`],
//! and its relationship types through [`transitional_type`].

use std::borrow::Cow;

/// The namespace of the `xml:` attributes (`xml:space`, `xml:lang`).
pub(crate) const XML: &str = "http://www.w3.org/XML/1998/namespace";
/// The namespace of namespace declarations.
pub(crate) const XMLNS: &str = "http://www.w3.org/2000/xmlns/";
/// XML Schema instance (`xsi:type` on a core property's date).
pub(crate) const XSI: &str = "http://www.w3.org/2001/XMLSchema-instance";

/// The content-types manifest, `[Content_Types].xml`.
pub(crate) const CONTENT_TYPES: &str =
    "http://schemas.openxmlformats.org/package/2006/content-types";
/// A relationships part (`*.rels`).
pub(crate) const RELATIONSHIPS: &str =
    "http://schemas.openxmlformats.org/package/2006/relationships";

/// Markup compatibility (`mc:`).
pub(crate) const MC: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";

/// OPC core properties (`cp:`).
pub(crate) const CORE_PROPERTIES: &str =
    "http://schemas.openxmlformats.org/package/2006/metadata/core-properties";
/// Dublin Core elements (`dc:`), used by the core properties.
pub(crate) const DC: &str = "http://purl.org/dc/elements/1.1/";
/// Dublin Core terms (`dcterms:`), used by the core properties.
pub(crate) const DCTERMS: &str = "http://purl.org/dc/terms/";
/// Dublin Core types (`dcmitype:`), declared on the core properties.
pub(crate) const DCMITYPE: &str = "http://purl.org/dc/dcmitype/";
/// Extended (app) properties.
pub(crate) const EXTENDED_PROPERTIES: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/extended-properties";
/// Custom properties.
pub(crate) const CUSTOM_PROPERTIES: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/custom-properties";
/// The variant types (`vt:`) the extended and custom properties hold.
pub(crate) const DOC_PROPS_VTYPES: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes";

/// The attributes (`r:id`, `r:embed`, `r:link`, …) by which an XML part names
/// one of its relationships.
pub(crate) const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

/// WordprocessingML (`w:`).
pub(crate) const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
/// The Word 2010 extensions (`w14:`).
pub(crate) const W14: &str = "http://schemas.microsoft.com/office/word/2010/wordml";
/// The Word 2012 extensions (`w15:`).
pub(crate) const W15: &str = "http://schemas.microsoft.com/office/word/2012/wordml";
/// The Word 2016 comment ids (`w16cid:`).
pub(crate) const W16CID: &str = "http://schemas.microsoft.com/office/word/2016/wordml/cid";
/// The Word 2018 extensible comments (`w16cex:`).
pub(crate) const W16CEX: &str = "http://schemas.microsoft.com/office/word/2018/wordml/cex";
/// PresentationML (`p:`).
pub(crate) const P: &str = "http://schemas.openxmlformats.org/presentationml/2006/main";
/// The PowerPoint 2010 extensions (`p14:`), whose slide creation ids name an
/// editing session.
pub(crate) const P14: &str = "http://schemas.microsoft.com/office/powerpoint/2010/main";
/// SpreadsheetML (`x:`, or the default namespace of a workbook's parts).
pub(crate) const X: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
/// The Excel 2010 application compatibility (`x15ac:`), which holds the path a
/// workbook was saved to.
pub(crate) const X15AC: &str = "http://schemas.microsoft.com/office/spreadsheetml/2010/11/ac";
/// The Excel 2014 revision ids (`xr:`).
pub(crate) const XR: &str = "http://schemas.microsoft.com/office/spreadsheetml/2014/revision";
/// The Excel 2015 revision ids (`xr2:`).
pub(crate) const XR2: &str = "http://schemas.microsoft.com/office/spreadsheetml/2015/revision2";
/// The Excel 2016 revision ids (`xr3:`).
pub(crate) const XR3: &str = "http://schemas.microsoft.com/office/spreadsheetml/2016/revision3";
/// The Excel 2016 revision ids (`xr6:`).
pub(crate) const XR6: &str = "http://schemas.microsoft.com/office/spreadsheetml/2016/revision6";
/// The Excel 2016 revision ids (`xr10:`).
pub(crate) const XR10: &str = "http://schemas.microsoft.com/office/spreadsheetml/2016/revision10";
/// Excel's threaded comments and their people.
pub(crate) const THREADED: &str =
    "http://schemas.microsoft.com/office/spreadsheetml/2018/threadedcomments";
/// PowerPoint's modern comments and their authors (`p188:`).
pub(crate) const P188: &str = "http://schemas.microsoft.com/office/powerpoint/2018/8/main";

/// DrawingML (`a:`).
pub(crate) const A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
/// DrawingML charts (`c:`).
pub(crate) const C: &str = "http://schemas.openxmlformats.org/drawingml/2006/chart";
/// DrawingML diagrams, SmartArt (`dgm:`).
pub(crate) const DGM: &str = "http://schemas.openxmlformats.org/drawingml/2006/diagram";
/// The Office 2014 charts, waterfall and sunburst among them (`cx:`).
pub(crate) const CX: &str = "http://schemas.microsoft.com/office/drawing/2014/chartex";
/// The Office 2012 chart styles and color styles (`cs:`).
pub(crate) const CS: &str = "http://schemas.microsoft.com/office/drawing/2012/chartStyle";
/// The Office 2008 SmartArt drawing, a diagram's shapes as drawn (`dsp:`).
pub(crate) const DSP: &str = "http://schemas.microsoft.com/office/drawing/2008/diagram";
/// DrawingML pictures (`pic:`).
pub(crate) const PIC: &str = "http://schemas.openxmlformats.org/drawingml/2006/picture";
/// DrawingML locked canvases (`lc:`).
pub(crate) const LC: &str = "http://schemas.openxmlformats.org/drawingml/2006/lockedCanvas";
/// A spreadsheet's drawing (`xdr:`).
pub(crate) const XDR: &str = "http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing";

/// Legacy VML shapes (`v:`).
pub(crate) const VML: &str = "urn:schemas-microsoft-com:vml";
/// Legacy Office drawing (`o:`).
pub(crate) const VML_OFFICE: &str = "urn:schemas-microsoft-com:office:office";
/// Legacy Excel drawing (`x:` in a VML drawing), whose `ClientData` says what a
/// shape is.
pub(crate) const VML_EXCEL: &str = "urn:schemas-microsoft-com:office:excel";
/// Legacy PowerPoint drawing (`p:` in a VML drawing).
pub(crate) const VML_POWERPOINT: &str = "urn:schemas-microsoft-com:office:powerpoint";
/// Legacy Word drawing (`w10:`).
pub(crate) const VML_WORD: &str = "urn:schemas-microsoft-com:office:word";

/// The strict namespaces and the transitional ones they map onto.
const STRICT: &[(&str, &str)] = &[
    ("http://purl.oclc.org/ooxml/officeDocument/relationships", R),
    (
        "http://purl.oclc.org/ooxml/officeDocument/math",
        "http://schemas.openxmlformats.org/officeDocument/2006/math",
    ),
    (
        "http://purl.oclc.org/ooxml/officeDocument/sharedTypes",
        "http://schemas.openxmlformats.org/officeDocument/2006/sharedTypes",
    ),
    (
        "http://purl.oclc.org/ooxml/officeDocument/extendedProperties",
        EXTENDED_PROPERTIES,
    ),
    (
        "http://purl.oclc.org/ooxml/officeDocument/customProperties",
        CUSTOM_PROPERTIES,
    ),
    (
        "http://purl.oclc.org/ooxml/officeDocument/docPropsVTypes",
        DOC_PROPS_VTYPES,
    ),
    ("http://purl.oclc.org/ooxml/wordprocessingml/main", W),
    ("http://purl.oclc.org/ooxml/presentationml/main", P),
    ("http://purl.oclc.org/ooxml/spreadsheetml/main", X),
    ("http://purl.oclc.org/ooxml/drawingml/main", A),
    (
        "http://purl.oclc.org/ooxml/drawingml/wordprocessingDrawing",
        "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing",
    ),
    (
        "http://purl.oclc.org/ooxml/drawingml/spreadsheetDrawing",
        XDR,
    ),
    ("http://purl.oclc.org/ooxml/drawingml/picture", PIC),
    ("http://purl.oclc.org/ooxml/drawingml/chart", C),
    (
        "http://purl.oclc.org/ooxml/drawingml/chartDrawing",
        "http://schemas.openxmlformats.org/drawingml/2006/chartDrawing",
    ),
    ("http://purl.oclc.org/ooxml/drawingml/diagram", DGM),
    ("http://purl.oclc.org/ooxml/drawingml/lockedCanvas", LC),
];

/// The transitional twin of the strict `namespace`, or `namespace` itself when
/// it is not a strict one.
pub(crate) fn transitional(namespace: &str) -> &str {
    STRICT
        .iter()
        .find(|(strict, _)| *strict == namespace)
        .map_or(namespace, |(_, transitional)| transitional)
}

/// Where the transitional relationship types are named.
const TRANSITIONAL_TYPES: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/";
/// Where their strict twins are, each under the same name.
const STRICT_TYPES: &str = "http://purl.oclc.org/ooxml/officeDocument/relationships/";

/// The transitional twin of the strict relationship type `kind`, or `kind`
/// itself when it is not a strict one.
pub(crate) fn transitional_type(kind: &str) -> Cow<'_, str> {
    match kind.strip_prefix(STRICT_TYPES) {
        Some(name) => Cow::Owned(format!("{TRANSITIONAL_TYPES}{name}")),
        None => Cow::Borrowed(kind),
    }
}
