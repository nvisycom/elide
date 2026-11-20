//! Extensions to lopdf's object model: the generic reads and writes the
//! crate's modules share, as methods on the types they read.

use elide_core::{Error, Result};
use lopdf::{
    DecompressError, Dictionary, Document, Error as PdfError, Object, ObjectId, Stream,
    decode_text_string,
};

/// The most any one content stream, page, or embedded image may decode to: a
/// compressed stream may claim any size, so its decode is bounded to refuse a
/// decompression bomb.
pub(crate) const MAX_CONTENT_BYTES: usize = 64 * 1024 * 1024;

/// How far a `/Parent` inheritance walk goes, against a cyclic page tree.
const MAX_PARENT_WALK: u32 = 64;

/// Reads and writes over a whole [`Document`].
pub(crate) trait DocumentExt {
    /// `object`, following a reference.
    fn resolve<'a>(&'a self, object: &'a Object) -> Option<&'a Object>;

    /// `object` as a dictionary, following a reference.
    fn resolve_dict<'a>(&'a self, object: &'a Object) -> Option<&'a Dictionary>;

    /// `object` as an array, following a reference.
    fn resolve_array<'a>(&'a self, object: &'a Object) -> Option<&'a Vec<Object>>;

    /// `object` as a number, following a reference; `None` when it is not one.
    fn resolve_number(&self, object: &Object) -> Option<f64>;

    /// `object` as an array of `N` numbers (a rectangle, a matrix),
    /// following references to it and its items; `None` when it is not one.
    fn resolve_numbers<const N: usize>(&self, object: &Object) -> Option<[f64; N]>;

    /// The value of the page attribute `key` on `page` or the nearest
    /// page-tree ancestor holding one `read` accepts.
    fn inherited<'a, T>(
        &'a self,
        page: ObjectId,
        key: &[u8],
        read: impl Fn(&'a Object) -> Option<T>,
    ) -> Option<T>;

    /// The decoded content of `page`, its content streams concatenated, at
    /// most [`MAX_CONTENT_BYTES`].
    ///
    /// # Errors
    ///
    /// [`ErrorKind::ResourceLimit`](elide_core::ErrorKind::ResourceLimit) if the
    /// content decodes past the bound, or
    /// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if a
    /// content stream is missing or cannot be decoded.
    fn page_content(&self, page: ObjectId) -> Result<Vec<u8>>;

    /// Write `content` as the decompressed body of the stream `id`, leaving its
    /// other keys intact.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if
    /// `id` is not a stream.
    fn set_stream_content(&mut self, id: ObjectId, content: Vec<u8>) -> Result<()>;
}

impl DocumentExt for Document {
    fn resolve<'a>(&'a self, object: &'a Object) -> Option<&'a Object> {
        match object {
            Object::Reference(id) => self.get_object(*id).ok(),
            other => Some(other),
        }
    }

    fn resolve_dict<'a>(&'a self, object: &'a Object) -> Option<&'a Dictionary> {
        self.resolve(object)?.as_dict().ok()
    }

    fn resolve_array<'a>(&'a self, object: &'a Object) -> Option<&'a Vec<Object>> {
        self.resolve(object)?.as_array().ok()
    }

    fn resolve_number(&self, object: &Object) -> Option<f64> {
        self.resolve(object)?.as_number()
    }

    fn resolve_numbers<const N: usize>(&self, object: &Object) -> Option<[f64; N]> {
        let items = <&[Object; N]>::try_from(self.resolve_array(object)?.as_slice()).ok()?;
        let mut numbers = [0.0; N];
        for (number, item) in numbers.iter_mut().zip(items) {
            *number = self.resolve_number(item)?;
        }
        Some(numbers)
    }

    fn inherited<'a, T>(
        &'a self,
        page: ObjectId,
        key: &[u8],
        read: impl Fn(&'a Object) -> Option<T>,
    ) -> Option<T> {
        let mut id = page;
        for _ in 0..MAX_PARENT_WALK {
            let dict = self.get_dictionary(id).ok()?;
            if let Some(value) = dict.get(key).ok().and_then(&read) {
                return Some(value);
            }
            id = dict.get(b"Parent").ok()?.as_reference().ok()?;
        }
        None
    }

    fn page_content(&self, page: ObjectId) -> Result<Vec<u8>> {
        let mut content = Vec::new();
        for id in self.get_page_contents(page) {
            let stream = self
                .get_object(id)
                .and_then(Object::as_stream)
                .map_err(|e| Error::malformed(format!("page content stream {id:?}: {e}")))?;
            content.extend(decode(
                stream,
                MAX_CONTENT_BYTES.saturating_sub(content.len()),
            )?);
            content.push(b'\n');
        }
        Ok(content)
    }

    fn set_stream_content(&mut self, id: ObjectId, content: Vec<u8>) -> Result<()> {
        match self.get_object_mut(id) {
            Ok(Object::Stream(stream)) => {
                stream.set_plain_content(content);
                Ok(())
            }
            _ => Err(Error::malformed(format!(
                "object {id:?} is not a stream to rewrite"
            ))),
        }
    }
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
    fn decoded(&self) -> Result<Vec<u8>>;

    /// As [`decoded`](Self::decoded), at most `limit` bytes.
    ///
    /// # Errors
    ///
    /// As [`decoded`](Self::decoded), past `limit`.
    fn decoded_within(&self, limit: usize) -> Result<Vec<u8>>;
}

impl StreamExt for Stream {
    fn decoded(&self) -> Result<Vec<u8>> {
        decode(self, MAX_CONTENT_BYTES)
    }

    fn decoded_within(&self, limit: usize) -> Result<Vec<u8>> {
        decode(self, limit)
    }
}

/// `stream`'s content with its filters decoded, at most `limit` bytes.
fn decode(stream: &Stream, limit: usize) -> Result<Vec<u8>> {
    // lopdf reads a stream whose filter is not a name or names as if it had
    // none, which would take its encoded bytes for its content.
    // A null entry is no entry.
    let filtered = stream
        .dict
        .get(b"Filter")
        .is_ok_and(|filter| !matches!(filter, Object::Null));
    if filtered && stream.filters().is_err() {
        return Err(Error::malformed("a stream's /Filter is not a filter name"));
    }
    stream
        .get_plain_content_with_limit(limit)
        .map_err(|e| match e {
            PdfError::Decompress(DecompressError::MemoryLimitExceeded { .. }) => {
                Error::resource_limit(format!("a stream decodes past its {limit}-byte limit"))
            }
            e => Error::malformed(format!("content stream: {e}")),
        })
}

/// Reads of one [`Object`].
pub(crate) trait ObjectExt {
    /// The object's dictionary: its own, or a stream's.
    fn dict(&self) -> Option<&Dictionary>;

    /// The object's dictionary, mutably: its own, or a stream's.
    fn dict_mut(&mut self) -> Option<&mut Dictionary>;

    /// Whether the object is a cross-reference stream: file structure the next
    /// save rewrites, never content.
    fn is_xref_stream(&self) -> bool;

    /// The object as a number, when it is an integer or a real.
    fn as_number(&self) -> Option<f64>;
}

impl ObjectExt for Object {
    fn dict(&self) -> Option<&Dictionary> {
        match self {
            Self::Dictionary(dict) => Some(dict),
            Self::Stream(stream) => Some(&stream.dict),
            _ => None,
        }
    }

    fn dict_mut(&mut self) -> Option<&mut Dictionary> {
        match self {
            Self::Dictionary(dict) => Some(dict),
            Self::Stream(stream) => Some(&mut stream.dict),
            _ => None,
        }
    }

    fn is_xref_stream(&self) -> bool {
        matches!(self, Self::Stream(stream) if stream.dict.has_type(b"XRef"))
    }

    fn as_number(&self) -> Option<f64> {
        match self {
            Self::Integer(i) => Some(*i as f64),
            Self::Real(r) => Some(f64::from(*r)),
            _ => None,
        }
    }
}

/// The keys a file specification names its file under, in the order they
/// are read: the Unicode name, the portable name, then the legacy platform
/// names (7.11.3).
const FILE_KEYS: [&[u8]; 5] = [b"UF", b"F", b"DOS", b"Mac", b"Unix"];

/// Reads and writes of one [`Dictionary`].
pub(crate) trait DictionaryExt {
    /// The name the dictionary holds under `key`, following a reference
    /// into `doc`.
    fn name<'a>(&'a self, doc: &'a Document, key: &[u8]) -> Option<&'a [u8]>;

    /// Whether the dictionary's `/Type` is `kind`.
    fn is_type(&self, doc: &Document, kind: &[u8]) -> bool;

    /// Whether the dictionary's `/Subtype` is `subtype`.
    fn is_subtype(&self, doc: &Document, subtype: &[u8]) -> bool;

    /// This file specification's file name: its Unicode `/UF`, else its
    /// `/F`, else a legacy platform name (`/DOS`, `/Mac`, `/Unix`).
    fn file_name(&self, doc: &Document) -> Option<String>;

    /// The embedded-file stream this file specification names in its `/EF`,
    /// under the first of the keys its name is read under that holds one,
    /// and the file's name: for the portable stream its `/UF`, else its
    /// `/F`; for a legacy platform stream its name under that same key;
    /// else its name.
    fn embedded_file(&self, doc: &Document) -> Option<(ObjectId, Option<String>)>;
}

impl DictionaryExt for Dictionary {
    fn name<'a>(&'a self, doc: &'a Document, key: &[u8]) -> Option<&'a [u8]> {
        let value = doc.resolve(self.get(key).ok()?)?;
        value.as_name().ok()
    }

    fn is_type(&self, doc: &Document, kind: &[u8]) -> bool {
        self.name(doc, b"Type") == Some(kind)
    }

    fn is_subtype(&self, doc: &Document, subtype: &[u8]) -> bool {
        self.name(doc, b"Subtype") == Some(subtype)
    }

    fn file_name(&self, doc: &Document) -> Option<String> {
        FILE_KEYS.iter().find_map(|key| {
            let name = doc.resolve(self.get(key).ok()?)?;
            decode_text_string(name).ok()
        })
    }

    fn embedded_file(&self, doc: &Document) -> Option<(ObjectId, Option<String>)> {
        let files = doc.resolve_dict(self.get(b"EF").ok()?)?;
        // The stream is named by the object its reference chain ends at, so
        // two references to one stream name it alike.
        let (key, stream) = FILE_KEYS.iter().find_map(|&key| {
            let reference = files.get(key).ok()?;
            let (Some(id), Object::Stream(_)) = doc.dereference(reference).ok()? else {
                return None;
            };
            Some((key, id))
        })?;
        // The portable stream (`/F` or `/UF`) is named by the Unicode name
        // first, as a viewer names it; a legacy platform stream by its own.
        let names: &[&[u8]] = match key {
            b"UF" | b"F" => &[b"UF", b"F"],
            key => &[key],
        };
        let named = names.iter().find_map(|key| {
            let name = doc.resolve(self.get(key).ok()?)?;
            decode_text_string(name).ok()
        });
        Some((stream, named.or_else(|| self.file_name(doc))))
    }
}
