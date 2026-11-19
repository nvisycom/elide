//! Reads over hayro-syntax's objects that the crate's modules share, as
//! methods on the types they read.

use elide_core::Result;
use hayro_syntax::object::{Array, Dict, MaybeRef, Name, ObjRef, Object, ObjectIdentifier, Stream};

use crate::pdf::decode;

/// The number and generation of an object in a document read.
pub(crate) type ObjectId = ObjectIdentifier;

/// The most any one content stream, page, or embedded image may decode to: a
/// compressed stream may claim any size, so its decode is bounded to refuse a
/// decompression bomb.
pub(crate) const MAX_CONTENT_BYTES: usize = 64 * 1024 * 1024;

/// How far a `/Parent` inheritance walk goes, against a cyclic page tree.
const MAX_PARENT_WALK: u32 = 64;

/// The keys a file specification names its file under, in the order they
/// are read: the Unicode name, the portable name, then the legacy platform
/// names (7.11.3).
const FILE_KEYS: [&[u8]; 5] = [b"UF", b"F", b"DOS", b"Mac", b"Unix"];

/// The object `reference` names.
pub(crate) fn id(reference: ObjRef) -> ObjectId {
    reference.into()
}

/// A reference to the object `id`, as a value is held.
pub(crate) fn reference_to<'a>(id: ObjectId) -> MaybeRef<Object<'a>> {
    MaybeRef::Ref(ObjRef::new(id.obj_number, id.gen_number))
}

/// Reads of one [`Dict`].
pub(crate) trait DictExt<'a> {
    /// The name under `key`.
    fn name(&self, key: &[u8]) -> Option<Name<'a>>;

    /// Whether `/Type` is `kind`.
    fn is_type(&self, kind: &[u8]) -> bool;

    /// Whether `/Subtype` is `subtype`.
    fn is_subtype(&self, subtype: &[u8]) -> bool;

    /// The number under `key`.
    fn number(&self, key: &[u8]) -> Option<f64>;

    /// The array of `N` numbers under `key` (a rectangle, a matrix).
    fn numbers<const N: usize>(&self, key: &[u8]) -> Option<[f64; N]>;

    /// The object `key` refers to, when it holds a reference.
    fn reference(&self, key: &[u8]) -> Option<ObjectId>;

    /// The value under `key`, as held: a reference, or the value in place.
    fn raw(&self, key: &[u8]) -> Option<MaybeRef<Object<'a>>>;

    /// The value of the page attribute `key` on this page or the nearest
    /// page-tree ancestor holding one `read` accepts.
    fn inherited<T>(&self, key: &[u8], read: impl Fn(&Dict<'a>) -> Option<T>) -> Option<T>;

    /// This file specification's file name: its Unicode `/UF`, else its
    /// `/F`, else a legacy platform name (`/DOS`, `/Mac`, `/Unix`).
    fn file_name(&self) -> Option<String>;

    /// The embedded-file stream this file specification names in its `/EF`,
    /// under the first of the keys its name is read under that holds one,
    /// and the file's name: for the portable stream its `/UF`, else its
    /// `/F`; for a legacy platform stream its name under that same key;
    /// else its name.
    fn embedded_file(&self) -> Option<(Stream<'a>, Option<String>)>;
}

impl<'a> DictExt<'a> for Dict<'a> {
    fn name(&self, key: &[u8]) -> Option<Name<'a>> {
        self.get::<Name<'a>>(key)
    }

    fn is_type(&self, kind: &[u8]) -> bool {
        self.name(b"Type").is_some_and(|name| &*name == kind)
    }

    fn is_subtype(&self, subtype: &[u8]) -> bool {
        self.name(b"Subtype").is_some_and(|name| &*name == subtype)
    }

    fn number(&self, key: &[u8]) -> Option<f64> {
        self.get::<f64>(key)
    }

    fn numbers<const N: usize>(&self, key: &[u8]) -> Option<[f64; N]> {
        numbers(&self.get::<Array<'a>>(key)?)
    }

    fn reference(&self, key: &[u8]) -> Option<ObjectId> {
        self.get_ref(key).map(id)
    }

    fn raw(&self, key: &[u8]) -> Option<MaybeRef<Object<'a>>> {
        self.get_raw::<Object<'a>>(key)
    }

    fn inherited<T>(&self, key: &[u8], read: impl Fn(&Dict<'a>) -> Option<T>) -> Option<T> {
        let mut node = self.clone();
        for _ in 0..MAX_PARENT_WALK {
            if node.contains_key(key)
                && let Some(value) = read(&node)
            {
                return Some(value);
            }
            node = node.get::<Dict<'a>>(b"Parent")?;
        }
        None
    }

    fn file_name(&self) -> Option<String> {
        FILE_KEYS.iter().find_map(|key| {
            let name = self.get::<hayro_syntax::object::String<'_>>(key)?;
            Some(text_string(name.as_bytes()))
        })
    }

    fn embedded_file(&self) -> Option<(Stream<'a>, Option<String>)> {
        let files = self.get::<Dict<'a>>(b"EF")?;
        let (key, stream) = FILE_KEYS
            .iter()
            .find_map(|&key| Some((key, files.get::<Stream<'a>>(key)?)))?;
        // The portable stream (`/F` or `/UF`) is named by the Unicode name
        // first, as a viewer names it; a legacy platform stream by its own.
        let names: &[&[u8]] = match key {
            b"UF" | b"F" => &[b"UF", b"F"],
            key => &[key],
        };
        let named = names.iter().find_map(|key| {
            let name = self.get::<hayro_syntax::object::String<'_>>(key)?;
            Some(text_string(name.as_bytes()))
        });
        Some((stream, named.or_else(|| self.file_name())))
    }
}

/// `array` as `N` numbers, when it holds exactly that many.
pub(crate) fn numbers<const N: usize>(array: &Array<'_>) -> Option<[f64; N]> {
    let mut out = [0.0; N];
    let mut items = array.iter::<Object<'_>>();
    for slot in &mut out {
        *slot = items.next()?.into_number()?.as_f64();
    }
    items.next().is_none().then_some(out)
}

/// Reads of one [`Stream`].
pub(crate) trait StreamExt {
    /// The stream's content with its filters decoded, at most
    /// [`MAX_CONTENT_BYTES`].
    ///
    /// # Errors
    ///
    /// [`ErrorKind::ResourceLimit`](elide_core::ErrorKind::ResourceLimit) if the
    /// content decodes past the bound, or
    /// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if
    /// it cannot be decoded.
    fn plain(&self) -> Result<Vec<u8>>;

    /// As [`plain`](Self::plain), at most `limit` bytes.
    ///
    /// # Errors
    ///
    /// As [`plain`](Self::plain), past `limit`.
    fn plain_within(&self, limit: usize) -> Result<Vec<u8>>;
}

impl StreamExt for Stream<'_> {
    fn plain(&self) -> Result<Vec<u8>> {
        decode::decode(self, MAX_CONTENT_BYTES)
    }

    fn plain_within(&self, limit: usize) -> Result<Vec<u8>> {
        decode::decode(self, limit)
    }
}

/// The decoded content of the page `page`, its content streams
/// concatenated, at most [`MAX_CONTENT_BYTES`].
///
/// # Errors
///
/// [`ErrorKind::ResourceLimit`](elide_core::ErrorKind::ResourceLimit) if the
/// content decodes past the bound, or
/// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if a
/// content stream cannot be decoded.
pub(crate) fn page_content(page: &Dict<'_>) -> Result<Vec<u8>> {
    let mut content = Vec::new();
    for stream in page_streams(page) {
        content.extend(stream.plain_within(MAX_CONTENT_BYTES.saturating_sub(content.len()))?);
        content.push(b'\n');
    }
    Ok(content)
}

/// The content streams of the page `page`, in order: its `/Contents`
/// stream, or each stream its array names.
pub(crate) fn page_streams<'a>(page: &Dict<'a>) -> Vec<Stream<'a>> {
    if let Some(stream) = page.get::<Stream<'a>>(b"Contents") {
        return vec![stream];
    }
    page.get::<Array<'a>>(b"Contents")
        .map(|streams| streams.iter::<Stream<'a>>().collect())
        .unwrap_or_default()
}

/// The ids of the content streams of the page `page`, as [`page_streams`].
pub(crate) fn page_stream_ids(page: &Dict<'_>) -> Vec<ObjectId> {
    page_streams(page).iter().map(Stream::obj_id).collect()
}

/// `bytes` read as a text string (7.9.2.2): UTF-16BE or UTF-8 after its
/// byte order mark, else PDFDocEncoding.
pub(crate) fn text_string(bytes: &[u8]) -> String {
    if let Some(utf16) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        let units = utf16
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&pair| u16::from_be_bytes(pair));
        return char::decode_utf16(units)
            .map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
            .collect();
    }
    if let Some(utf8) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(utf8).into_owned();
    }
    bytes.iter().map(|&b| pdf_doc_char(b)).collect()
}

/// The character the PDFDocEncoding byte `byte` stands for (Annex D.2):
/// Latin-1, but for the bytes it gives other characters.
fn pdf_doc_char(byte: u8) -> char {
    const LOW: [char; 8] = ['˘', 'ˇ', 'ˆ', '˙', '˝', '˛', '˚', '˜'];
    const HIGH: [char; 32] = [
        '•', '†', '‡', '…', '—', '–', 'ƒ', '⁄', '‹', '›', '−', '‰', '„', '“', '”', '‘', '’', '‚',
        '™', 'ﬁ', 'ﬂ', 'Ł', 'Œ', 'Š', 'Ÿ', 'Ž', 'ı', 'ł', 'œ', 'š', 'ž', '\u{FFFD}',
    ];
    match byte {
        0x18..=0x1F => LOW[usize::from(byte - 0x18)],
        0x80..=0x9F => HIGH[usize::from(byte - 0x80)],
        0xA0 => '€',
        byte => char::from(byte),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_strings_decode_by_their_mark() {
        assert_eq!(text_string(b"\xFE\xFF\x00h\x00i"), "hi");
        assert_eq!(text_string(b"\xEF\xBB\xBFh\xC3\xA9"), "hé");
        assert_eq!(text_string(b"a\x84b\xA0"), "a—b€");
        assert_eq!(text_string(b"caf\xE9"), "café");
    }
}
