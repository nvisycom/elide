//! [`Inspection`]: a bounded, fail-closed inventory of a PDF's risk-bearing
//! structures and how completely it could be inspected.
//!
//! Inspection walks the current object graph — never a superseded revision —
//! and reports two things: a [`RiskInventory`] of the structures that can
//! retain sensitive data (forms, annotations, attachments, metadata, scripts,
//! signatures, …), and the [`Coverage`] it achieved. It reads the document; it
//! never modifies it, and it needs no renderer. A caller uses it to decide
//! whether a document can be safely redacted and by which path.

mod coverage;
mod risk;

use lopdf::Object;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

pub use self::coverage::{Coverage, CoverageGap, CoverageStatus};
pub use self::risk::RiskInventory;
use crate::Pdf;
use crate::error::{Error, Result};

/// A bounded inventory of a PDF's risk-bearing structures and inspection
/// coverage, produced by [`Pdf::inspect`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(rename_all = "camelCase")
)]
#[non_exhaustive]
pub struct Inspection {
    /// The document's PDF version string (e.g. `"1.5"`).
    pub pdf_version: String,
    /// Number of indirect objects in the current object graph.
    pub object_count: u32,
    /// Number of pages.
    pub page_count: u32,
    /// Whether the document is encrypted.
    pub encrypted: bool,
    /// Tally of structures that can retain sensitive data.
    pub risks: RiskInventory,
    /// How completely the document could be inspected.
    pub coverage: Coverage,
}

impl Inspection {
    /// Maximum number of indirect objects an inspected document may hold. A
    /// larger graph is refused by [`Pdf::inspect`] rather than walked unbounded.
    pub const MAX_OBJECTS: usize = 200_000;
    /// Maximum number of pages an inspected document may hold.
    pub const MAX_PAGES: usize = 10_000;
}

impl Pdf {
    /// Inspect the document: inventory its risk-bearing structures and report
    /// how completely it could be inspected.
    ///
    /// This reads the current object graph only. An encrypted document, a
    /// retained prior revision, or bytes after the final `%%EOF` are recorded
    /// as [coverage gaps](CoverageGap) rather than silently ignored, so a
    /// caller never treats an incomplete inspection as a clean one.
    ///
    /// Inspection alone is **not** anonymisation: it says what sensitive
    /// structures exist, not that they have been removed.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::LimitExceeded`](crate::ErrorKind::LimitExceeded) if the
    /// document exceeds the object-count ([`MAX_OBJECTS`](Inspection::MAX_OBJECTS))
    /// or page-count ([`MAX_PAGES`](Inspection::MAX_PAGES)) bound, refused
    /// rather than walked unboundedly.
    pub fn inspect(&self) -> Result<Inspection> {
        let object_count = self.doc.objects.len();
        if object_count > Inspection::MAX_OBJECTS {
            return Err(Error::limit_exceeded(format!(
                "document has {object_count} objects, over the {}-object limit",
                Inspection::MAX_OBJECTS
            )));
        }
        let pages = self.doc.get_pages();
        if pages.len() > Inspection::MAX_PAGES {
            return Err(Error::limit_exceeded(format!(
                "document has {} pages, over the {}-page limit",
                pages.len(),
                Inspection::MAX_PAGES
            )));
        }

        let encrypted = self.doc.is_encrypted();
        let mut risks = self.risk_inventory();

        // Retained-bytes accounting works on the raw source, independent of the
        // parsed graph: extra `%%EOF` markers mean superseded revisions, and
        // non-whitespace bytes after the last one are unaccounted content.
        let (revisions, trailing) = retained_bytes(&self.source_bytes());
        risks.incremental_revision_count = revisions;
        risks.trailing_non_whitespace_byte_count = trailing;

        let mut gaps = Vec::new();
        if encrypted {
            gaps.push(CoverageGap::EncryptedDocument);
        }
        if revisions > 0 || trailing > 0 {
            gaps.push(CoverageGap::RetainedDocumentBytes);
        }

        Ok(Inspection {
            pdf_version: self.doc.version.clone(),
            object_count: object_count.min(u32::MAX as usize) as u32,
            page_count: pages.len().min(u32::MAX as usize) as u32,
            encrypted,
            risks,
            coverage: Coverage::from_gaps(gaps),
        })
    }

    /// Walk the object graph once and tally every risk-bearing structure.
    fn risk_inventory(&self) -> RiskInventory {
        let mut risks = RiskInventory::default();

        // Trailer-rooted structures.
        if let Ok(info) = self
            .doc
            .trailer
            .get(b"Info")
            .and_then(Object::as_reference)
            .and_then(|id| self.doc.get_object(id))
            .and_then(Object::as_dict)
        {
            risks.document_info_entry_count = info.len().min(u32::MAX as usize) as u32;
        }
        if let Ok(form) = self
            .doc
            .catalog()
            .and_then(|catalog| catalog.get(b"AcroForm"))
            .and_then(|o| self.resolve_dict(o))
        {
            if let Ok(fields) = form.get(b"Fields").and_then(Object::as_array) {
                risks.acro_form_field_count = fields.len().min(u32::MAX as usize) as u32;
            }
            if form.has(b"XFA") {
                risks.xfa_entry_count = xfa_entry_count(form.get(b"XFA").ok());
            }
        }

        // Per-object classification: one pass over the whole graph.
        for object in self.doc.objects.values() {
            let Ok(dict) = object
                .as_dict()
                .or_else(|_| object.as_stream().map(|s| &s.dict))
            else {
                continue;
            };
            let ty = dict.get(b"Type").and_then(Object::as_name).ok();
            let subtype = dict.get(b"Subtype").and_then(Object::as_name).ok();

            match ty {
                Some(b"Metadata") => risks.metadata_stream_count += 1,
                Some(b"Annot") => risks.annotation_count += 1,
                Some(b"Sig") => risks.signature_count += 1,
                Some(b"Filespec") | Some(b"EmbeddedFile") => risks.embedded_file_count += 1,
                Some(b"OCG") => risks.optional_content_group_count += 1,
                _ => {}
            }
            match subtype {
                Some(b"Image") => risks.image_object_count += 1,
                Some(b"Form") => risks.form_x_object_count += 1,
                _ => {}
            }

            // Action dictionaries: classify by `/S` (action subtype).
            if dict.has(b"S")
                && (dict.has(b"URI")
                    || dict.has(b"JS")
                    || dict.has(b"F")
                    || matches!(ty, Some(b"Action")))
            {
                match dict.get(b"S").and_then(Object::as_name).ok() {
                    Some(b"JavaScript") => risks.javascript_action_count += 1,
                    Some(b"URI") | Some(b"Launch") | Some(b"GoToR") | Some(b"SubmitForm") => {
                        risks.external_action_count += 1
                    }
                    Some(_) => risks.unsupported_action_count += 1,
                    None => {}
                }
            }
        }

        risks
    }

    /// Resolve an object that may be an indirect reference to its dictionary.
    fn resolve_dict<'a>(&'a self, object: &'a Object) -> lopdf::Result<&'a lopdf::Dictionary> {
        match object {
            Object::Reference(id) => self.doc.get_object(*id).and_then(Object::as_dict),
            other => other.as_dict(),
        }
    }
}

/// Count XFA entries: an XFA value is either a stream or an array of
/// `(name, stream)` pairs. A present XFA counts at least one.
fn xfa_entry_count(xfa: Option<&Object>) -> u32 {
    match xfa {
        Some(Object::Array(items)) => (items.len() / 2).max(1).min(u32::MAX as usize) as u32,
        Some(_) => 1,
        None => 0,
    }
}

/// Scan the raw bytes for retained content: the number of *superseded*
/// incremental revisions, and the count of non-whitespace bytes after the final
/// `%%EOF`.
///
/// Each `%%EOF` marker ends a cross-reference section. A single-revision file
/// has one; each incremental update appends another. **Linearization** is the
/// exception: a web-optimised file has a first-page cross-reference section with
/// its own `%%EOF` plus the main one — two markers, but a single revision — so
/// that extra marker is not counted as a superseded revision.
fn retained_bytes(source: &[u8]) -> (u32, u64) {
    const EOF: &[u8] = b"%%EOF";
    // Only the marker count and the last marker's position are needed, so track
    // those directly rather than retaining every position.
    let mut markers = 0usize;
    let mut last_marker = 0usize;
    let mut i = 0;
    while i + EOF.len() <= source.len() {
        if &source[i..i + EOF.len()] == EOF {
            markers += 1;
            last_marker = i;
            i += EOF.len();
        } else {
            i += 1;
        }
    }
    if markers == 0 {
        return (0, 0);
    }

    // `%%EOF` markers beyond the first mark appended revisions — but a
    // linearized file's first-page section adds one `%%EOF` that is part of the
    // same (single) revision, so discount it.
    let linearization_markers = usize::from(is_linearized(source));
    let revisions = markers
        .saturating_sub(1)
        .saturating_sub(linearization_markers)
        .min(u32::MAX as usize) as u32;

    let last = last_marker + EOF.len();
    let trailing = source[last..]
        .iter()
        .filter(|b| !b.is_ascii_whitespace())
        .count() as u64;
    (revisions, trailing)
}

/// Whether the document is linearized: its first object dictionary carries the
/// `/Linearized` key, which appears near the start of the file.
fn is_linearized(source: &[u8]) -> bool {
    // The linearization dictionary is required to be within the first 1024 bytes
    // of the file (right after the header); scan a generous prefix.
    let prefix_len = source.len().min(2048);
    source[..prefix_len]
        .windows(b"/Linearized".len())
        .any(|w| w == b"/Linearized")
}

#[cfg(test)]
mod tests {
    use super::retained_bytes;

    #[test]
    fn two_eof_markers_are_one_incremental_revision() {
        let bytes = b"%PDF-1.5\n... body ...\n%%EOF\n... update ...\n%%EOF\n";
        let (revisions, trailing) = retained_bytes(bytes);
        assert_eq!(revisions, 1, "a second %%EOF is an appended revision");
        assert_eq!(trailing, 0);
    }

    #[test]
    fn linearized_two_eof_markers_are_a_single_revision() {
        // A linearized file: `/Linearized` near the start, two `%%EOF` markers
        // (first-page section + main) — but one revision.
        let bytes = b"%PDF-1.5\n1 0 obj<</Linearized 1>>endobj\n... first page ...\n\
                      %%EOF\n... main body ...\n%%EOF\n";
        let (revisions, _) = retained_bytes(bytes);
        assert_eq!(
            revisions, 0,
            "linearization's extra %%EOF is not a revision"
        );
    }

    #[test]
    fn trailing_bytes_after_final_eof_are_counted() {
        let bytes = b"%PDF-1.5\nbody\n%%EOF\nLEFTOVER";
        let (revisions, trailing) = retained_bytes(bytes);
        assert_eq!(revisions, 0);
        assert_eq!(trailing, "LEFTOVER".len() as u64);
    }
}
