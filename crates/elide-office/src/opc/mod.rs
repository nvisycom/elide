//! The Office Open XML packaging (OPC) engine: a zip of parts, opened once,
//! extracted, and rewritten in place, format-neutral.
//!
//! [`Package`] is the shared core every OOXML format (DOCX, and, ahead, XLSX,
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
#[cfg(feature = "test-util")]
pub mod test_util;
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

/// The largest total uncompressed size a package may inflate to across all its
/// parts. The per-part cap alone lets an archive of many large entries exhaust
/// memory, since [`open`](Package::open) retains every part; this bounds the sum
/// so a bomb of many parts is refused before it can. 1 GiB comfortably exceeds
/// any legitimate OOXML document while staying well within process memory.
const MAX_PACKAGE_BYTES: u64 = 1024 * 1024 * 1024;

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
    /// exist"), a format facade layers that on top.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::InvalidArchive`](crate::ErrorKind::InvalidArchive) if the
    /// bytes are not a readable zip, a part is unreadable, or a part exceeds the
    /// size cap.
    pub fn open(document: &[u8], classifier: C) -> Result<Self> {
        Self::open_with_limits(document, classifier, MAX_PART_BYTES, MAX_PACKAGE_BYTES)
    }

    /// [`open`](Package::open) with explicit `part_cap` (per-part) and
    /// `package_cap` (aggregate) byte limits, so tests can exercise the caps with
    /// a small archive.
    fn open_with_limits(
        document: &[u8],
        classifier: C,
        part_cap: u64,
        package_cap: u64,
    ) -> Result<Self> {
        let mut zip = ZipArchive::new(Cursor::new(document))
            .map_err(|e| Error::invalid_archive(format!("not a zip: {e}")))?;

        let mut parts = Vec::with_capacity(zip.len());
        // Bytes still allowed across the whole package; each part's read and
        // allocation are bounded by this, so a bomb of many parts is refused
        // before it can inflate past the package cap.
        let mut budget = package_cap;
        for i in 0..zip.len() {
            let entry = zip
                .by_index(i)
                .map_err(|e| Error::invalid_archive(format!("bad zip entry: {e}")))?;
            let path = PartPath::from(entry.name());
            let role = classifier.role(&path);
            // Cap this part at the smaller of the per-part limit and what remains
            // of the package budget. Reserve only up to that cap (the entry's
            // claimed size may lie), and bound the read one byte past it so an
            // over-large part is detected rather than inflated.
            let cap = part_cap.min(budget);
            let claimed = entry.size().min(cap);
            let mut buf = Vec::with_capacity(claimed as usize);
            let read = entry
                .take(cap + 1)
                .read_to_end(&mut buf)
                .map_err(|e| Error::invalid_archive(format!("part `{path}` unreadable: {e}")))?;
            if read as u64 > cap {
                return Err(Error::invalid_archive(format!(
                    "package exceeds size limits at part `{path}` \
                     (part cap {part_cap}, package cap {package_cap} bytes)"
                )));
            }
            budget -= read as u64;
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

    /// The raw bytes of the part at `path`, or `None` when the package has no
    /// such part. A cheap ref-counted share of the retained buffer, so a facade
    /// can parse a part's own structure (e.g. XLSX reading its shared-string
    /// table and sheet cells) before deciding what to redact.
    pub fn part_bytes(&self, path: &str) -> Option<Bytes> {
        self.parts
            .iter()
            .find(|p| p.path().as_str() == path)
            .map(StoredPart::bytes)
    }

    /// The paths of every part, in archive order, so a facade can discover its
    /// parts (e.g. XLSX enumerating `xl/worksheets/sheet*.xml`) without assuming
    /// fixed names.
    pub fn part_paths(&self) -> impl Iterator<Item = &PartPath> {
        self.parts.iter().map(StoredPart::path)
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

        // Index whole-part replacements, validating each names an existing part
        // that is redactable (never a structure/metadata part), not protected,
        // and not already targeted by a text splice.
        let mut part_bytes: HashMap<&PartPath, &[u8]> = HashMap::new();
        for pr in parts {
            let Some(part) = index.get(&pr.part) else {
                return Err(Error::unsafe_rewrite(format!(
                    "part replacement names unknown part `{}`",
                    pr.part
                )));
            };
            // A whole-part replacement may only overwrite a redactable part, a
            // binary embedding, or a text part redacted out of band. Refusing a
            // `Structure` part closes the hole where redacted bytes could
            // overwrite styles, the theme, or the content-types manifest.
            if part.role() == PartRole::Structure {
                return Err(Error::unsafe_rewrite(format!(
                    "part replacement targets non-redactable part `{}`",
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

#[cfg(test)]
mod tests {
    use std::io::Write;

    use zip::write::SimpleFileOptions;
    use zip::{CompressionMethod, ZipWriter};

    use super::*;

    /// A classifier that routes one text part and one media part, marking the
    /// text part protected, so every role and guard can be exercised.
    struct TestClassifier;

    impl PartClassifier for TestClassifier {
        fn role(&self, path: &PartPath) -> PartRole {
            match path.as_str() {
                "doc/text.xml" => PartRole::ElementText,
                "media/image1.png" => PartRole::Binary(EmbeddingKind::Image),
                _ => PartRole::Structure,
            }
        }

        fn is_protected(&self, path: &PartPath) -> bool {
            path.as_str() == "doc/text.xml"
        }
    }

    /// A zip of `(name, bytes)` entries.
    fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        for (name, bytes) in entries {
            zip.start_file(*name, opts).unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    #[test]
    fn a_part_replacement_targeting_a_structure_part_is_refused() {
        let bytes = zip_of(&[("styles.xml", b"<styles/>"), ("doc/text.xml", b"<t>hi</t>")]);
        let package = Package::open(&bytes, TestClassifier).unwrap();
        // `styles.xml` is a Structure part: replacing its bytes wholesale must be
        // refused, so redacted bytes can't overwrite the package's structure.
        let replacement = PartReplacement::new(PartPath::from("styles.xml"), b"<evil/>".to_vec());
        assert!(package.rewrite_with_parts(&[], &[replacement]).is_err());
    }

    #[test]
    fn a_part_replacement_on_a_binary_part_is_accepted() {
        let bytes = zip_of(&[
            ("media/image1.png", b"\x89PNG original"),
            ("doc/text.xml", b"<t>hi</t>"),
        ]);
        let package = Package::open(&bytes, TestClassifier).unwrap();
        let replacement = PartReplacement::new(
            PartPath::from("media/image1.png"),
            b"\x89PNG redacted".to_vec(),
        );
        let out = package.rewrite_with_parts(&[], &[replacement]).unwrap();
        let repacked = Package::open(&out, TestClassifier).unwrap();
        assert_eq!(
            repacked.part_bytes("media/image1.png").unwrap().as_ref(),
            b"\x89PNG redacted"
        );
    }

    #[test]
    fn a_package_exceeding_the_aggregate_size_limit_is_refused() {
        // Two parts each within the per-part cap but together over the package
        // cap: the second crosses the aggregate budget and is refused, even
        // though neither part alone is too large. Tiny caps keep the archive
        // small.
        let bytes = zip_of(&[("a.xml", &[b'x'; 40]), ("b.xml", &[b'y'; 40])]);
        // Per-part cap 100 (each 40-byte part is fine); package cap 64 (the two
        // parts sum to 80, so the second overflows).
        let err = Package::open_with_limits(&bytes, TestClassifier, 100, 64);
        assert!(
            err.is_err(),
            "aggregate over-budget package must be refused"
        );

        // The same archive opens when the package cap admits both parts.
        assert!(Package::open_with_limits(&bytes, TestClassifier, 100, 100).is_ok());
    }

    #[test]
    fn a_single_part_over_the_per_part_cap_is_refused() {
        let bytes = zip_of(&[("a.xml", &[b'x'; 200])]);
        // Per-part cap 100 rejects the 200-byte part; package cap is generous.
        assert!(Package::open_with_limits(&bytes, TestClassifier, 100, 10_000).is_err());
    }
}
