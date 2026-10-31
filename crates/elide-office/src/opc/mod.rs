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

pub(crate) mod archive;
mod attributes;
mod block;
pub(crate) mod check;
pub(crate) mod content_types;
#[cfg(any(test, feature = "fixtures"))]
pub mod fixtures;
pub(crate) mod index;
pub(crate) mod kind;
pub(crate) mod names;
mod offset;
mod part;
pub mod props;
pub(crate) mod references;
pub(crate) mod relationships;
pub(crate) mod sanitize;
mod store;
pub(crate) mod vocabulary;
pub(crate) mod xml_edit;
pub(crate) mod xml_span;

use std::collections::HashMap;

use bytes::Bytes;
use elide_core::{Error, ErrorKind, Result};

pub use self::attributes::TextAttribute;
pub(crate) use self::attributes::{NEUTRAL_DATE, NeutralizeAttributes, Ns, StripAttributes};
pub use self::block::{
    Block, Embedding, EmbeddingKind, Extraction, Issue, IssueKind, PartReplacement, Replacement,
};
use self::index::Index;
pub use self::kind::PartKind;
pub use self::names::{NameReference, NameRule, Names};
pub use self::offset::{OffsetMap, OffsetRun, RunKind};
pub use self::part::{PartClassifier, PartPath, PartRole};
use self::store::StoredPart;

/// An opened OOXML package: every part read once and classified, ready to
/// [`extract`](Package::extract) the text of every text-bearing part or
/// [`rewrite`](Package::rewrite) them back to bytes.
///
/// Open a document once and reuse it for both operations; the package is parsed
/// a single time.
#[derive(Debug, Clone)]
pub struct Package {
    /// Every package part, in archive order.
    parts: Vec<StoredPart>,
    /// The format's names, fitted to their rules when redacted.
    names: &'static Names,
    /// The original archive bytes, retained so [`rewrite_with_parts`](Self::rewrite_with_parts) can
    /// copy an untouched part's already-compressed data straight through instead
    /// of inflating and re-deflating it.
    ///
    /// Bounded by the same open-time caps as the parts: deflate does not expand
    /// content, so the compressed archive is within the package cap (plus small
    /// per-entry zip overhead) of the decompressed parts already retained. It
    /// adds at most a second copy of the already-in-memory input, never an
    /// unbounded allocation.
    source: Bytes,
}

impl Package {
    /// Open a package from its bytes, reading and classifying every part (by
    /// its content type) and tagging it with the role `classifier` assigns its
    /// kind.
    ///
    /// This is the neutral open: it validates the zip and reads every part, but
    /// applies no format-specific structural requirement (e.g. "a main part must
    /// exist"), a format facade layers that on top.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::MalformedInput`] if the bytes are not a readable zip, the
    /// content-types manifest or a relationships part is unreadable, a part is
    /// unreadable, or a part exceeds the size cap.
    pub fn open(document: &[u8], classifier: &dyn PartClassifier) -> Result<Self> {
        Self::open_with_limits(
            document,
            classifier,
            archive::MAX_PART_BYTES,
            archive::MAX_PACKAGE_BYTES,
        )
    }

    /// [`open`](Package::open) with explicit `part_cap` (per-part) and
    /// `package_cap` (aggregate) byte limits, so tests can exercise the caps with
    /// a small archive.
    fn open_with_limits(
        document: &[u8],
        classifier: &dyn PartClassifier,
        part_cap: u64,
        package_cap: u64,
    ) -> Result<Self> {
        let index = Index::read_with_limits(document, part_cap, package_cap)?;
        let parts = index
            .parts
            .iter()
            .map(|(path, bytes)| {
                let kind = index.kind(path);
                let attributes = index.is_xml(path).then(|| classifier.text_attributes());
                StoredPart::new(
                    path.clone(),
                    kind,
                    classifier.role(kind),
                    bytes.clone(),
                    attributes,
                )
            })
            .collect();
        Ok(Self {
            parts,
            names: classifier.names(),
            source: Bytes::copy_from_slice(document),
        })
    }

    /// Whether the package contains a part of `kind`, so a facade can enforce a
    /// format's required part (e.g. a Word document must have its main part).
    pub fn contains_kind(&self, kind: PartKind) -> bool {
        self.parts.iter().any(|p| p.kind() == kind)
    }

    /// Whether the package contains a part at `path`.
    pub fn contains_part(&self, path: &str) -> bool {
        self.parts.iter().any(|p| p.path().as_str() == path)
    }

    /// The role of the part at `path`, or `None` when the package has no such
    /// part.
    pub fn role_of(&self, path: &PartPath) -> Option<PartRole> {
        self.parts
            .iter()
            .find(|p| p.path() == path)
            .map(StoredPart::role)
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
        let mut issues = Vec::new();
        for part in &self.parts {
            if part.is_text() {
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
            }
        }
        Extraction {
            blocks,
            embeddings: self.embeddings(),
            issues,
        }
    }

    /// The binary embeddings (images, media, embedded objects) the classifier
    /// surfaces for redaction, each addressed by its part.
    pub fn embeddings(&self) -> Vec<Embedding> {
        self.parts
            .iter()
            .filter_map(|part| match part.role() {
                PartRole::Binary(kind) => Some(Embedding {
                    part: part.path().clone(),
                    kind,
                    bytes: part.bytes(),
                }),
                _ => None,
            })
            .collect()
    }

    /// Rewrite text `replacements` across their parts and re-pack every other
    /// part byte-for-byte.
    ///
    /// See [`rewrite_with_parts`](Package::rewrite_with_parts) to also replace
    /// binary parts (e.g. redact an embedded image).
    ///
    /// **Fail-closed:** an out-of-bounds, overlapping, or mid-character
    /// replacement, or one naming a part not in the package, refuses the whole
    /// rewrite with [`ErrorKind::Processing`]
    /// rather than emitting a partially-redacted document.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Processing`] if a
    /// replacement can't be applied.
    /// The text of the part at `path` with `replacements` spliced in, for a
    /// facade that rewrites the part further before handing it back whole.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Processing`] if the package has no such text part or a
    /// replacement does not land on one of its text spans.
    pub(crate) fn splice_part(
        &self,
        path: &PartPath,
        replacements: &[&Replacement],
    ) -> Result<String> {
        let part = self
            .parts
            .iter()
            .find(|part| part.path() == path && part.is_text())
            .ok_or_else(|| {
                Error::new(
                    ErrorKind::Processing,
                    format!("replacement names non-text part `{path}`"),
                )
            })?;
        part.splice(replacements)
    }

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
        let replacements = self.fit_names(replacements)?;
        self.rewrite_fitted(&replacements, parts)
    }

    /// `replacements` with each one landing in a name of the format folded into
    /// a replacement of the whole name fitted to its rules, and the references
    /// to a renamed name rewritten (see [`names`]).
    ///
    /// # Errors
    ///
    /// [`ErrorKind::MalformedInput`] if a text part is not UTF-8 or well-formed
    /// XML.
    pub(crate) fn fit_names(&self, replacements: &[Replacement]) -> Result<Vec<Replacement>> {
        let mut texts = Vec::new();
        for part in &self.parts {
            if !part.is_text() || part.role() == PartRole::RelationshipTargets {
                continue;
            }
            let text = part.text().ok_or_else(|| {
                Error::new(
                    ErrorKind::MalformedInput,
                    format!("part `{}` not UTF-8", part.path()),
                )
            })?;
            texts.push((part.path(), text));
        }
        names::fit_names(self.names, texts.iter().copied(), replacements)
    }

    /// [`rewrite_with_parts`](Self::rewrite_with_parts) with the names already
    /// fitted.
    pub(crate) fn rewrite_fitted(
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
                return Err(Error::new(
                    ErrorKind::Processing,
                    format!("replacement names unknown part `{}`", r.part),
                ));
            };
            if !part.is_text() {
                return Err(Error::new(
                    ErrorKind::Processing,
                    format!("replacement names non-text part `{}`", r.part),
                ));
            }
            by_part.entry(&r.part).or_default().push(r);
        }

        // Index whole-part replacements, validating each names an existing part
        // whose role admits a whole replacement and that is not already targeted
        // by a text splice.
        let mut part_bytes: HashMap<&PartPath, &[u8]> = HashMap::new();
        for pr in parts {
            let Some(part) = index.get(&pr.part) else {
                return Err(Error::new(
                    ErrorKind::Processing,
                    format!("part replacement names unknown part `{}`", pr.part),
                ));
            };
            // A whole-part replacement may only overwrite a part whose role
            // admits it: a binary embedding, a property part, or a cell part.
            // Refusing every other role closes the hole where redacted bytes
            // could overwrite the main part, styles, the theme, the content-types
            // manifest, or splice a text part out of band.
            if !part.role().is_whole_part_replaceable() {
                return Err(Error::new(
                    ErrorKind::Processing,
                    format!("part replacement targets non-redactable part `{}`", pr.part),
                ));
            }
            if by_part.contains_key(&pr.part) {
                return Err(Error::new(
                    ErrorKind::Processing,
                    format!(
                        "part `{}` has both a text splice and a binary replacement",
                        pr.part
                    ),
                ));
            }
            part_bytes.insert(&pr.part, &pr.bytes);
        }

        // Re-pack in archive order: a changed part (spliced text or replaced
        // bytes) is re-deflated; an untouched part is copied straight from the
        // source with its data still compressed, no inflate/deflate round-trip.
        let mut changed: Vec<(&PartPath, Vec<u8>)> = Vec::new();
        for part in &self.parts {
            match by_part.get(part.path()) {
                Some(edits) => changed.push((part.path(), part.splice(edits)?.into_bytes())),
                None => {
                    if let Some(bytes) = part_bytes.get(part.path()) {
                        changed.push((part.path(), bytes.to_vec()));
                    }
                }
            }
        }
        let entries = self.parts.iter().map(|part| {
            match changed.iter().find(|(path, _)| *path == part.path()) {
                Some((path, bytes)) => archive::Entry::Changed(path, bytes),
                None => archive::Entry::Unchanged(part.path()),
            }
        });
        archive::write(&self.source, entries)
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write};

    use zip::write::SimpleFileOptions;
    use zip::{CompressionMethod, ZipWriter};

    use super::*;

    /// A classifier that makes the main part text, an image binary, and the
    /// rest structure, so every replacement guard can be exercised.
    struct TestClassifier;

    impl PartClassifier for TestClassifier {
        fn role(&self, kind: PartKind) -> PartRole {
            match kind {
                PartKind::Main => PartRole::ElementText,
                PartKind::Embedding(kind) => PartRole::Binary(kind),
                _ => PartRole::Structure,
            }
        }
    }

    /// A package of a main text part, a style part, and an image, with its
    /// content-types manifest.
    fn package() -> Vec<u8> {
        let manifest = br#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="png" ContentType="image/png"/><Override PartName="/doc/text.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/></Types>"#;
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        let entries: [(&str, &[u8]); 4] = [
            ("[Content_Types].xml", manifest),
            ("doc/text.xml", b"<t>hi</t>"),
            ("styles.xml", b"<styles/>"),
            ("media/image1.png", b"\x89PNG original"),
        ];
        for (name, bytes) in entries {
            zip.start_file(name, opts).unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    #[test]
    fn a_part_replacement_targeting_structure_or_text_is_refused() {
        let package = Package::open(&package(), &TestClassifier).unwrap();
        // Neither a structure part nor the main text part may be overwritten
        // wholesale, so redacted bytes can't replace the package's structure.
        for path in ["styles.xml", "doc/text.xml", "[Content_Types].xml"] {
            let replacement = PartReplacement::new(PartPath::from(path), b"<evil/>".to_vec());
            assert!(
                package.rewrite_with_parts(&[], &[replacement]).is_err(),
                "{path}"
            );
        }
    }

    #[test]
    fn a_part_replacement_on_a_binary_part_is_accepted() {
        let package = Package::open(&package(), &TestClassifier).unwrap();
        let replacement = PartReplacement::new(
            PartPath::from("media/image1.png"),
            b"\x89PNG redacted".to_vec(),
        );
        let out = package.rewrite_with_parts(&[], &[replacement]).unwrap();
        let repacked = Package::open(&out, &TestClassifier).unwrap();
        assert_eq!(
            repacked.part_bytes("media/image1.png").unwrap().as_ref(),
            b"\x89PNG redacted"
        );
    }

    #[test]
    fn parts_are_classified_by_content_type() {
        let package = Package::open(&package(), &TestClassifier).unwrap();
        assert!(package.contains_kind(PartKind::Main));
        let extraction = package.extract();
        assert_eq!(extraction.blocks.len(), 1);
        assert_eq!(extraction.embeddings.len(), 1);
    }
}
