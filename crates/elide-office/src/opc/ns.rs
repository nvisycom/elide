//! The XML namespaces named in more than one place, each by the prefix Office
//! writes it under where it has a well-known one. The transitional URIs are
//! listed; a strict part's namespaces map onto these through
//! [`schema::transitional`](crate::opc::schema::transitional).

/// The namespace of the `xml:` attributes (`xml:space`, `xml:lang`).
pub(crate) const XML: &str = "http://www.w3.org/XML/1998/namespace";
/// The namespace of namespace declarations.
pub(crate) const XMLNS: &str = "http://www.w3.org/2000/xmlns/";
/// XML Schema instance (`xsi:type` on a core property's date).
pub(crate) const XSI: &str = "http://www.w3.org/2001/XMLSchema-instance";

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
/// SpreadsheetML (`x:`, or the default namespace of a workbook's parts).
pub(crate) const X: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";

/// DrawingML (`a:`).
pub(crate) const A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
/// DrawingML charts (`c:`).
pub(crate) const C: &str = "http://schemas.openxmlformats.org/drawingml/2006/chart";
/// DrawingML diagrams, SmartArt (`dgm:`).
pub(crate) const DGM: &str = "http://schemas.openxmlformats.org/drawingml/2006/diagram";
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
