//! Test-only readers for an OPC package's parts.
//!
//! Reading a part out of a rewritten package means reversing the same
//! zip container the engine writes. Downstream crates test redaction by
//! asserting on individual parts, and doing so soundly means
//! *decompressing* each part — a plaintext value can straddle the deflate
//! stream, so a substring check over the raw container bytes would give a
//! false pass. This module owns that decompression so a test never has to
//! reconstruct the packaging itself.
//!
//! Gated behind the `test-util` feature: it is scaffolding for tests in
//! this crate and its dependents, not part of the shipped surface.

use std::io::{Cursor, Read};

use zip::ZipArchive;

/// Read one part out of an OPC package by its full part name (e.g.
/// `word/document.xml`), decompressed. Returns `None` if the package
/// cannot be opened or the part is absent.
pub fn read_part(package: &[u8], name: &str) -> Option<Vec<u8>> {
    let mut zip = ZipArchive::new(Cursor::new(package)).ok()?;
    let mut entry = zip.by_name(name).ok()?;
    let mut buf = Vec::new();
    entry.read_to_end(&mut buf).ok()?;
    Some(buf)
}

/// Every part name in the package, in archive order. Useful to sweep the
/// whole package for leaked values.
pub fn part_names(package: &[u8]) -> Vec<String> {
    let mut zip = match ZipArchive::new(Cursor::new(package)) {
        Ok(zip) => zip,
        Err(_) => return Vec::new(),
    };
    (0..zip.len())
        .filter_map(|i| zip.by_index(i).ok().map(|e| e.name().to_owned()))
        .collect()
}

/// Every text-bearing part — the `.xml` and `.rels` parts — decompressed
/// to `(name, text)`. These parts carry all of a document's text; binary
/// parts (fonts, images, media) hold none and are skipped, so a leak scan
/// over the result covers every part that could carry text PII.
///
/// A text part that is not valid UTF-8 **panics** rather than being
/// dropped: silently omitting a part a leak scan can't read would let PII
/// in that part pass unseen — the same false-pass this module exists to
/// prevent. UTF-8 is also the only encoding the package engine itself
/// accepts (a non-UTF-8 part fails extraction as `NotUtf8`), so an
/// undecodable part here is a malformed fixture, not an encoding the
/// product supports.
pub fn text_parts(package: &[u8]) -> Vec<(String, String)> {
    let mut zip = ZipArchive::new(Cursor::new(package)).expect("package is a valid zip container");
    (0..zip.len())
        .filter_map(|i| {
            let mut entry = zip.by_index(i).expect("zip entry index in range");
            let name = entry.name().to_owned();
            if !(name.ends_with(".xml") || name.ends_with(".rels")) {
                return None;
            }
            let mut bytes = Vec::new();
            entry
                .read_to_end(&mut bytes)
                .unwrap_or_else(|e| panic!("read part `{name}`: {e}"));
            let text = String::from_utf8(bytes)
                .unwrap_or_else(|_| panic!("text part `{name}` is not valid UTF-8"));
            Some((name, text))
        })
        .collect()
}
