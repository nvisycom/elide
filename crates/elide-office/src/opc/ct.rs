//! The content types named in more than one place, lowercased as a part's
//! content type is compared.

/// The prefix of every WordprocessingML content type.
pub(crate) const WML: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.";
/// The prefix of every PresentationML content type.
pub(crate) const PML: &str = "application/vnd.openxmlformats-officedocument.presentationml.";
/// The prefix of every SpreadsheetML content type.
pub(crate) const SML: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.";
/// The prefix of the DrawingML part content types (charts, SmartArt).
pub(crate) const DML: &str = "application/vnd.openxmlformats-officedocument.drawingml.";

/// A legacy VML drawing: XML that predates the `+xml` naming.
pub(crate) const VML_DRAWING: &str = "application/vnd.openxmlformats-officedocument.vmldrawing";

/// Whether `content_type` is [`VML_DRAWING`], compared ASCII
/// case-insensitively.
pub(crate) fn is_vml_drawing(content_type: &str) -> bool {
    content_type.eq_ignore_ascii_case(VML_DRAWING)
}
