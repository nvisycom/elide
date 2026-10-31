//! The package's zip container: read every part under size bounds, and write a
//! package back, copying each untouched part's compressed bytes straight through.

use std::fmt::Display;
use std::io::{Cursor, Read, Write};

use bytes::Bytes;
use elide_core::{Error, ErrorKind, Result};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::opc::part::PartPath;

/// The largest a single package part may be. A zip entry may claim any
/// uncompressed size, so extraction is capped and the read is bounded to this
/// many bytes to refuse allocation-DoS and zip-bomb parts. 512 MiB comfortably
/// exceeds any legitimate OOXML part while staying far below memory a
/// decompressed bomb would demand.
pub(crate) const MAX_PART_BYTES: u64 = 512 * 1024 * 1024;

/// The largest total uncompressed size a package may inflate to across all its
/// parts. The per-part cap alone lets an archive of many large entries exhaust
/// memory, since every part is retained; this bounds the sum so a bomb of many
/// parts is refused before it can. 1 GiB comfortably exceeds any legitimate
/// OOXML document while staying well within process memory.
pub(crate) const MAX_PACKAGE_BYTES: u64 = 1024 * 1024 * 1024;

/// Read every part of the zip `document`, in archive order, refusing a part
/// past `part_cap` bytes or a package past `package_cap` in total.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`] if the bytes are not a readable zip, a part is
/// unreadable, or a size cap is exceeded.
pub(crate) fn read(
    document: &[u8],
    part_cap: u64,
    package_cap: u64,
) -> Result<Vec<(PartPath, Bytes)>> {
    let mut zip = ZipArchive::new(Cursor::new(document))
        .map_err(|e| Error::new(ErrorKind::MalformedInput, format!("not a zip: {e}")))?;
    let mut parts = Vec::with_capacity(zip.len());
    // Bytes still allowed across the whole package; each part's read and
    // allocation are bounded by this, so a bomb of many parts is refused before
    // it can inflate past the package cap.
    let mut budget = package_cap;
    for i in 0..zip.len() {
        let entry = zip
            .by_index(i)
            .map_err(|e| Error::new(ErrorKind::MalformedInput, format!("bad zip entry: {e}")))?;
        let name = entry.name().map_err(|e| {
            Error::new(
                ErrorKind::MalformedInput,
                format!("bad zip entry name: {e}"),
            )
        })?;
        let path = PartPath::from(name.as_ref());
        // Cap this part at the smaller of the per-part limit and what remains of
        // the package budget. Reserve only up to that cap (the entry's claimed
        // size may lie), and bound the read one byte past it so an over-large
        // part is detected rather than inflated.
        let cap = part_cap.min(budget);
        let claimed = entry.size().min(cap);
        let mut buf = Vec::with_capacity(claimed as usize);
        let read = entry.take(cap + 1).read_to_end(&mut buf).map_err(|e| {
            Error::new(
                ErrorKind::MalformedInput,
                format!("part `{path}` unreadable: {e}"),
            )
        })?;
        if read as u64 > cap {
            return Err(Error::new(
                ErrorKind::MalformedInput,
                format!(
                    "package exceeds size limits at part `{path}` \
                     (part cap {part_cap}, package cap {package_cap} bytes)"
                ),
            ));
        }
        budget -= read as u64;
        parts.push((path, Bytes::from(buf)));
    }
    Ok(parts)
}

/// One part of a package being written: copied from the source archive as it
/// was, or written with new bytes.
pub(crate) enum Entry<'a> {
    /// Copy the source archive's entry, still compressed.
    Unchanged(&'a PartPath),
    /// Write the part with these bytes.
    Changed(&'a PartPath, &'a [u8]),
}

/// Write a package of `entries`, in order, copying each unchanged one's
/// compressed bytes from the `source` archive.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`] if the source cannot be reopened or the
/// package cannot be written.
pub(crate) fn write<'a>(
    source: &[u8],
    entries: impl IntoIterator<Item = Entry<'a>>,
) -> Result<Vec<u8>> {
    let mut source = ZipArchive::new(Cursor::new(source))
        .map_err(|e| Error::new(ErrorKind::MalformedInput, format!("reopen source: {e}")))?;
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for entry in entries {
        let (path, bytes) = match entry {
            Entry::Unchanged(path) => match source.by_name(path.as_str()) {
                Ok(raw) => {
                    zip.raw_copy_file(raw).map_err(|e| repack(path, e))?;
                    continue;
                }
                Err(e) => return Err(repack(path, e)),
            },
            Entry::Changed(path, bytes) => (path, bytes),
        };
        zip.start_file(path.as_str(), options)
            .map_err(|e| repack(path, e))?;
        zip.write_all(bytes).map_err(|e| repack(path, e))?;
    }
    let cursor = zip
        .finish()
        .map_err(|e| Error::new(ErrorKind::MalformedInput, format!("repack failed: {e}")))?;
    Ok(cursor.into_inner())
}

/// The error for a part that could not be repacked.
fn repack(path: &PartPath, e: impl Display) -> Error {
    Error::new(ErrorKind::MalformedInput, format!("repack `{path}`: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A zip of `(name, bytes)` entries.
    fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        for (name, bytes) in entries {
            zip.start_file(*name, options).unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    #[test]
    fn a_package_exceeding_the_aggregate_size_limit_is_refused() {
        // Two parts each within the per-part cap but together over the package
        // cap: the second crosses the aggregate budget and is refused.
        let bytes = zip_of(&[("a.xml", &[b'x'; 40]), ("b.xml", &[b'y'; 40])]);
        assert!(read(&bytes, 100, 64).is_err());
        // The same archive reads when the package cap admits both parts.
        assert!(read(&bytes, 100, 100).is_ok());
    }

    #[test]
    fn a_single_part_over_the_per_part_cap_is_refused() {
        let bytes = zip_of(&[("a.xml", &[b'x'; 200])]);
        assert!(read(&bytes, 100, 10_000).is_err());
    }

    #[test]
    fn write_copies_unchanged_parts_and_writes_changed_ones() {
        let bytes = zip_of(&[("a.xml", b"<a/>"), ("b.xml", b"<b/>")]);
        let a = PartPath::from("a.xml");
        let b = PartPath::from("b.xml");
        let out = write(&bytes, [Entry::Unchanged(&a), Entry::Changed(&b, b"<B/>")]).unwrap();
        let parts = read(&out, 100, 1000).unwrap();
        assert_eq!(parts[0].1.as_ref(), b"<a/>");
        assert_eq!(parts[1].1.as_ref(), b"<B/>");
    }
}
