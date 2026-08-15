//! [`RiskInventory`]: a count of the document structures that can retain
//! sensitive data.
//!
//! A rich PDF holds personal data in more places than its page text: form
//! fields, annotations, embedded attachments, metadata, JavaScript, XFA, and
//! more. Inspection walks the object graph and tallies each such structure, so
//! a caller sees *what kinds of retained data exist* before deciding how to
//! sanitise — a text-layer rewrite that ignores these leaves them intact.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// A count of the document structures that can retain sensitive data.
///
/// Each field is a tally, not a location: a non-zero count means that class of
/// retained data is present. A rewrite that only edits page text does not touch
/// any of these, so a non-zero count is a redaction gap to reason about.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(rename_all = "camelCase")
)]
#[non_exhaustive]
pub struct RiskInventory {
    /// AcroForm field dictionaries (interactive form fields).
    pub acro_form_field_count: u32,
    /// Page annotations (comments, links, widgets, redaction marks).
    pub annotation_count: u32,
    /// Entries in the document information dictionary (`/Info`: author, title…).
    pub document_info_entry_count: u32,
    /// Embedded file attachments.
    pub embedded_file_count: u32,
    /// External actions (URI / launch / GoToR) that can leak or fetch.
    pub external_action_count: u32,
    /// Form XObjects (reusable content groups that may carry text/images).
    pub form_x_object_count: u32,
    /// Image XObjects.
    pub image_object_count: u32,
    /// Retained incremental revisions (superseded content still in the bytes).
    pub incremental_revision_count: u32,
    /// JavaScript actions.
    pub javascript_action_count: u32,
    /// Metadata streams (`/Metadata`, XMP).
    pub metadata_stream_count: u32,
    /// Optional content groups (layers).
    pub optional_content_group_count: u32,
    /// Digital signature dictionaries.
    pub signature_count: u32,
    /// Non-whitespace bytes after the final `%%EOF` marker.
    pub trailing_non_whitespace_byte_count: u64,
    /// Actions of a kind this inspector does not classify.
    pub unsupported_action_count: u32,
    /// XFA form entries (XML forms architecture).
    pub xfa_entry_count: u32,
}
