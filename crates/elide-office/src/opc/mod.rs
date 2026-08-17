//! The Office Open XML packaging (OPC) engine: a zip of parts, opened once,
//! extracted, and rewritten in place — format-neutral.
//!
//! [`Package`] is the shared core every OOXML format (DOCX, and — ahead — XLSX,
//! PPTX) builds on. A format supplies a [`PartClassifier`] that assigns each
//! part a [`PartRole`]; the engine acts on the role alone, so it never needs to
//! know one format's part schema from another's. The engine extracts the
//! redactable text of every text-bearing part (each block addressed by its part
//! and an exact byte span) and rewrites those spans back into a byte-faithful
//! package.

mod block;
mod offset;
mod part;
mod store;
mod xml_span;

use std::collections::HashMap;
use std::io::{Cursor, Read, Write};

use bytes::Bytes;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

pub use self::block::{
    Block, Embedding, EmbeddingKind, Extraction, Issue, IssueKind, PartReplacement, Replacement,
};
pub use self::offset::{OffsetMap, OffsetRun, RunKind};
pub use self::part::{PartClassifier, PartPath, PartRole};
use self::store::StoredPart;
use crate::error::{Error, Result};

/// The largest a single package part may be. A zip entry may claim any
/// uncompressed size, so extraction is capped and the read is bounded to this
/// many bytes to refuse allocation-DoS and zip-bomb parts. 512 MiB comfortably
/// exceeds any legitimate OOXML part while staying far below memory a
/// decompressed bomb would demand.
const MAX_PART_BYTES: u64 = 512 * 1024 * 1024;

/// An opened OOXML package: every part read once and classified by the
/// format's `C`, ready to [`extract`](Package::extract) the text of every
/// text-bearing part or [`rewrite`](Package::rewrite) them back to bytes.
///
/// Open a document once and reuse it for both operations; the package is parsed
/// a single time.
#[derive(Debug, Clone)]
pub struct Package<C: PartClassifier> {
    /// Every package part, in archive order.
    parts: Vec<StoredPart>,
    /// The format's part classifier, retained for rewrite-time protection
    /// checks.
    classifier: C,
}

impl<C: PartClassifier> Package<C> {
    /// Open a package from its bytes, reading every part and tagging it with the
    /// role `classifier` assigns.
    ///
    /// This is the neutral open: it validates the zip and reads every part, but
    /// applies no format-specific structural requirement (e.g. "a body part must
    /// exist") — a format facade layers that on top.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::InvalidArchive`](crate::ErrorKind::InvalidArchive) if the
    /// bytes are not a readable zip, a part is unreadable, or a part exceeds the
    /// size cap.
    pub fn open(document: &[u8], classifier: C) -> Result<Self> {
        let mut zip = ZipArchive::new(Cursor::new(document))
            .map_err(|e| Error::invalid_archive(format!("not a zip: {e}")))?;

        let mut parts = Vec::with_capacity(zip.len());
        for i in 0..zip.len() {
            let entry = zip
                .by_index(i)
                .map_err(|e| Error::invalid_archive(format!("bad zip entry: {e}")))?;
            let path = PartPath::from(entry.name());
            let role = classifier.role(&path);
            // Reserve only up to the cap (the entry's claimed size may lie), and
            // bound the read so a zip bomb cannot inflate past it.
            let claimed = entry.size().min(MAX_PART_BYTES);
            let mut buf = Vec::with_capacity(claimed as usize);
            let read = entry
                .take(MAX_PART_BYTES + 1)
                .read_to_end(&mut buf)
                .map_err(|e| Error::invalid_archive(format!("part `{path}` unreadable: {e}")))?;
            if read as u64 > MAX_PART_BYTES {
                return Err(Error::invalid_archive(format!(
                    "part `{path}` exceeds {MAX_PART_BYTES}-byte limit"
                )));
            }
            parts.push(StoredPart::new(path, role, Bytes::from(buf)));
        }

        Ok(Self { parts, classifier })
    }

    /// Whether the package contains a part at `path`, so a facade can enforce a
    /// format's required part (e.g. a Word document must have
    /// `word/document.xml`).
    pub fn contains_part(&self, path: &str) -> bool {
        self.parts.iter().any(|p| p.path().as_str() == path)
    }

    /// Extract the redactable text and embedded media of the package.
    ///
    /// Each [`Block`] is addressed by its part and an exact byte span into that
    /// part's XML; each [`Embedding`] by its part. Structure and metadata parts
    /// are carried through untouched. Extraction is partial-success: a text part
    /// that cannot be parsed is recorded in [`issues`](Extraction::issues)
    /// rather than failing the whole document.
    pub fn extract(&self) -> Extraction {
        let mut blocks = Vec::new();
        let mut embeddings = Vec::new();
        let mut issues = Vec::new();

        for part in &self.parts {
            let role = part.role();
            if role.is_redactable() {
                // Partial-success: a part that fails to parse yields no blocks
                // and is recorded as an issue rather than failing the whole
                // extraction.
                match part.text_blocks() {
                    Ok(part_blocks) => blocks.extend(part_blocks),
                    Err(kind) => issues.push(Issue {
                        part: part.path().clone(),
                        kind,
                    }),
                }
            } else if let PartRole::Binary(kind) = role {
                embeddings.push(Embedding {
                    part: part.path().clone(),
                    kind,
                    bytes: part.bytes(),
                });
            }
        }

        Extraction {
            blocks,
            embeddings,
            issues,
        }
    }

    /// Rewrite text `replacements` across their parts and re-pack every other
    /// part byte-for-byte.
    ///
    /// See [`rewrite_with_parts`](Package::rewrite_with_parts) to also replace
    /// binary parts (e.g. redact an embedded image).
    ///
    /// **Fail-closed:** an out-of-bounds, overlapping, or mid-character
    /// replacement, or one naming a part not in the package, refuses the whole
    /// rewrite with [`ErrorKind::UnsafeRewrite`](crate::ErrorKind::UnsafeRewrite)
    /// rather than emitting a partially-redacted document.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::UnsafeRewrite`](crate::ErrorKind::UnsafeRewrite) if a
    /// replacement can't be applied.
    pub fn rewrite(&self, replacements: &[Replacement]) -> Result<Vec<u8>> {
        self.rewrite_with_parts(replacements, &[])
    }

    /// Rewrite text `replacements` *and* replace binary `parts` (each a part
    /// path mapped to its new bytes).
    ///
    /// A [`PartReplacement`] naming a part not in the package refuses the
    /// rewrite; the text rules match [`rewrite`](Package::rewrite).
    ///
    /// # Errors
    ///
    /// As [`rewrite`](Package::rewrite).
    pub fn rewrite_with_parts(
        &self,
        replacements: &[Replacement],
        parts: &[PartReplacement],
    ) -> Result<Vec<u8>> {
        // Index every stored part once for O(1) lookup during validation.
        let index: HashMap<&PartPath, &StoredPart> =
            self.parts.iter().map(|p| (p.path(), p)).collect();

        // Group text replacements by part, validating each names an existing
        // text-bearing part.
        let mut by_part: HashMap<&PartPath, Vec<&Replacement>> = HashMap::new();
        for r in replacements {
            let Some(part) = index.get(&r.part) else {
                return Err(Error::unsafe_rewrite(format!(
                    "replacement names unknown part `{}`",
                    r.part
                )));
            };
            if !part.role().is_redactable() {
                return Err(Error::unsafe_rewrite(format!(
                    "replacement names non-text part `{}`",
                    r.part
                )));
            }
            by_part.entry(&r.part).or_default().push(r);
        }

        // Index binary part replacements, validating each names an existing part
        // that is neither structural nor already targeted by a text splice.
        let mut part_bytes: HashMap<&PartPath, &[u8]> = HashMap::new();
        for pr in parts {
            if !index.contains_key(&pr.part) {
                return Err(Error::unsafe_rewrite(format!(
                    "part replacement names unknown part `{}`",
                    pr.part
                )));
            }
            if self.classifier.is_protected(&pr.part) {
                return Err(Error::unsafe_rewrite(format!(
                    "part replacement targets protected structural part `{}`",
                    pr.part
                )));
            }
            if by_part.contains_key(&pr.part) {
                return Err(Error::unsafe_rewrite(format!(
                    "part `{}` has both a text splice and a binary replacement",
                    pr.part
                )));
            }
            part_bytes.insert(&pr.part, &pr.bytes);
        }

        // Re-pack: each part gets its spliced text, its replaced bytes, or its
        // original bytes, in archive order.
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        for part in &self.parts {
            let bytes: Vec<u8> = if let Some(edits) = by_part.get(part.path()) {
                part.splice(edits)?.into_bytes()
            } else if let Some(replaced) = part_bytes.get(part.path()) {
                replaced.to_vec()
            } else {
                part.bytes().to_vec()
            };
            let fail = |e: String| Error::invalid_package(format!("repack `{}`: {e}", part.path()));
            zip.start_file(part.path().as_str(), opts)
                .map_err(|e| fail(e.to_string()))?;
            zip.write_all(&bytes).map_err(|e| fail(e.to_string()))?;
        }
        let cursor = zip
            .finish()
            .map_err(|e| Error::invalid_package(format!("repack failed: {e}")))?;
        Ok(cursor.into_inner())
    }
}
