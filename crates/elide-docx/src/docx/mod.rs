//! [`Docx`]: an opened DOCX package, extracted and rewritten in place.

mod part_store;

use std::collections::HashMap;
use std::io::{Cursor, Read, Write};

use bytes::Bytes;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use self::part_store::StoredPart;
use crate::block::{Embedding, Extraction, Issue, PartReplacement, Replacement};
use crate::error::{Error, Result};
use crate::part::{PartKind, PartPath};

/// The largest a single package part may be. A zip entry may claim any
/// uncompressed size, so extraction is capped and the read is bounded to this
/// many bytes to refuse allocation-DoS and zip-bomb parts. 512 MiB comfortably
/// exceeds any legitimate WordprocessingML part while staying far below memory
/// a decompressed bomb would demand.
const MAX_PART_BYTES: u64 = 512 * 1024 * 1024;

/// An opened DOCX package: every part read once and classified, ready to
/// [`extract`](Docx::extract) the text of every text-bearing part or
/// [`rewrite`](Docx::rewrite) them back to bytes.
///
/// Open a document once and reuse it for both operations; the package is parsed
/// a single time.
#[derive(Debug, Clone)]
pub struct Docx {
    /// Every package part, in archive order.
    parts: Vec<StoredPart>,
}

impl Docx {
    /// Open a DOCX from its bytes, reading and classifying every part.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidArchive`](crate::ErrorKind::InvalidArchive) if the
    ///   bytes are not a zip;
    /// - [`ErrorKind::InvalidPackage`](crate::ErrorKind::InvalidPackage) if the
    ///   body part is missing.
    pub fn open(document: &[u8]) -> Result<Self> {
        let mut zip = ZipArchive::new(Cursor::new(document))
            .map_err(|e| Error::invalid_archive(format!("not a zip: {e}")))?;

        let mut parts = Vec::with_capacity(zip.len());
        let mut has_body = false;
        for i in 0..zip.len() {
            let entry = zip
                .by_index(i)
                .map_err(|e| Error::invalid_archive(format!("bad zip entry: {e}")))?;
            let path = PartPath::from(entry.name());
            has_body |= path.kind() == PartKind::Body;
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
            parts.push(StoredPart::new(path, Bytes::from(buf)));
        }

        if !has_body {
            return Err(Error::invalid_package(
                "missing body part `word/document.xml`",
            ));
        }
        Ok(Self { parts })
    }

    /// Extract the redactable text and embedded images of the document.
    ///
    /// Each [`Block`](crate::block::Block) is addressed by its part and an exact byte
    /// span into that part's XML; each [`Embedding`](crate::block::Embedding) by its
    /// part. Metadata and structure parts are carried through untouched.
    /// Extraction is partial-success: a text part that cannot be parsed is
    /// recorded in [`issues`](Extraction::issues) rather than failing the whole
    /// document.
    pub fn extract(&self) -> Extraction {
        let mut blocks = Vec::new();
        let mut embeddings = Vec::new();
        let mut issues = Vec::new();

        for part in &self.parts {
            if part.kind().is_text() {
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
            } else if let Some(kind) = part.kind().embedding() {
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
    /// See [`rewrite_with_parts`](Docx::rewrite_with_parts) to also replace
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
    /// A [`PartReplacement`](crate::block::PartReplacement) naming a part not in the package refuses the
    /// rewrite; the text rules match [`rewrite`](Docx::rewrite).
    ///
    /// # Errors
    ///
    /// As [`rewrite`](Docx::rewrite).
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
            if !part.kind().is_text() {
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
            if pr.part.is_protected() {
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
