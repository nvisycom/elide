//! Extensions to lopdf's object model: the generic reads and writes the
//! crate's modules share, as methods on the types they read.

use elide_core::{Error, Result};
use lopdf::{Dictionary, Document, Object, ObjectId, decode_text_string};

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

    /// The value of the page attribute `key` on `page` or the nearest
    /// page-tree ancestor holding one `read` accepts.
    fn inherited<T>(
        &self,
        page: ObjectId,
        key: &[u8],
        read: impl Fn(&Object) -> Option<T>,
    ) -> Option<T>;

    /// The resource dictionaries `page`'s content resolves names against: its
    /// own and those it inherits.
    fn page_resources(&self, page: ObjectId) -> Vec<Dictionary>;

    /// Write `content` as the decompressed body of the stream `id`, leaving its
    /// other keys intact.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if
    /// `id` is not a stream.
    fn set_stream_content(&mut self, id: ObjectId, content: Vec<u8>) -> Result<()>;

    /// The document serialized, first dropping every object it no longer
    /// reaches: a superseded content stream, a replaced image's mask, a stale
    /// cross-reference stream. lopdf serializes an unreachable object too, so
    /// without the prune a redacted original would survive in the bytes.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if
    /// the document cannot be serialized.
    fn save_pruned(&mut self) -> Result<Vec<u8>>;
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

    fn inherited<T>(
        &self,
        page: ObjectId,
        key: &[u8],
        read: impl Fn(&Object) -> Option<T>,
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

    fn page_resources(&self, page: ObjectId) -> Vec<Dictionary> {
        let Ok((own, inherited)) = self.get_page_resources(page) else {
            return Vec::new();
        };
        own.cloned()
            .into_iter()
            .chain(
                inherited
                    .into_iter()
                    .filter_map(|id| self.get_dictionary(id).ok().cloned()),
            )
            .collect()
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

    fn save_pruned(&mut self) -> Result<Vec<u8>> {
        self.objects.retain(|_, object| !object.is_xref_stream());
        self.prune_objects();
        let mut out = Vec::new();
        self.save_to(&mut out)
            .map_err(|e| Error::malformed(format!("could not save PDF: {e}")))?;
        Ok(out)
    }
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

/// Reads and writes of one [`Dictionary`].
pub(crate) trait DictionaryExt {
    /// Whether the dictionary's `/Subtype` is `subtype`.
    fn has_subtype(&self, subtype: &[u8]) -> bool;

    /// Remove `key`, returning 1 when it was present and 0 otherwise, for a
    /// caller counting what it removed.
    fn remove_counted(&mut self, key: &[u8]) -> usize;

    /// This file specification's file name: its Unicode `/UF`, else its `/F`.
    fn file_name(&self) -> Option<String>;
}

impl DictionaryExt for Dictionary {
    fn has_subtype(&self, subtype: &[u8]) -> bool {
        self.get(b"Subtype")
            .and_then(Object::as_name)
            .is_ok_and(|name| name == subtype)
    }

    fn remove_counted(&mut self, key: &[u8]) -> usize {
        usize::from(self.remove(key).is_some())
    }

    fn file_name(&self) -> Option<String> {
        self.get(b"UF")
            .or_else(|_| self.get(b"F"))
            .ok()
            .and_then(|name| decode_text_string(name).ok())
    }
}
