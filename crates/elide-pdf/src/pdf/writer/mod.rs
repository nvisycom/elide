//! Writing a PDF: a [`Document`] built object by object and saved through
//! pdf-writer, and an [`Edit`] of an opened one, which copies it with the
//! objects each redaction substitutes.

mod cmap;
mod content;
mod document;
mod name_tree;

use std::collections::{BTreeMap, HashSet};
use std::io::Write as _;

use elide_core::{Error, Result};
use flate2::Compression;
use flate2::write::ZlibEncoder;
use hayro_syntax::PdfVersion;
use hayro_syntax::object::{Dict, MaybeRef, Object, Stream as Source};

pub(crate) use self::cmap::{UnicodeMap, scrub};
pub(crate) use self::content::{drop_xobjects, rewrite_contents, write_page_content};
pub(crate) use self::document::{Document, write as write_value};
pub(crate) use self::name_tree::retain_names;
use super::object::{Dictionary, Id, Stream, Value};
use super::reader::ObjectId;
use super::{Pdf, Redact};

/// A stream of `dict` holding `content`, compressed.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if
/// `content` cannot be compressed.
pub(crate) fn compressed(mut dict: Dictionary, content: &[u8]) -> Result<Stream> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(content)
        .map_err(|e| Error::malformed(format!("could not compress a stream: {e}")))?;
    let data = encoder
        .finish()
        .map_err(|e| Error::malformed(format!("could not compress a stream: {e}")))?;
    dict.set("Filter", "FlateDecode");
    dict.remove(b"DecodeParms");
    Ok(Stream::new(dict, data))
}

/// The document `doc` saved and opened again.
///
/// # Errors
///
/// As [`Document::save`], and
/// [`ErrorKind::ResourceLimit`](elide_core::ErrorKind::ResourceLimit) if it
/// saves past [`Pdf::MAX_DOCUMENT_BYTES`].
pub(crate) fn finish(doc: &Document) -> Result<Pdf> {
    let bytes = doc.save()?;
    // What is written is opened again (to check it, or to redact it), so it
    // is held to the limit any document is opened within.
    if bytes.len() > Pdf::MAX_DOCUMENT_BYTES {
        return Err(Error::resource_limit(format!(
            "the document written is {} bytes, over the {}-byte limit",
            bytes.len(),
            Pdf::MAX_DOCUMENT_BYTES
        )));
    }
    Pdf::open_bytes(bytes.into())
}

/// The object of the document written that the object `id` read is copied
/// as: the same number, as an edit keeps every object's.
pub(crate) fn copied(id: ObjectId) -> Id {
    Id(id.obj_number)
}

/// The value `item` read, held as it is: a reference stays one, to the
/// object it is [`copied`] as.
pub(crate) fn value(item: &MaybeRef<Object<'_>>) -> Value {
    match item {
        MaybeRef::Ref(reference) => Value::Ref(copied((*reference).into())),
        MaybeRef::NotRef(object) => object_value(object),
    }
}

/// The value `object` read, held in place.
pub(crate) fn object_value(object: &Object<'_>) -> Value {
    match object {
        Object::Null(_) => Value::Null,
        Object::Boolean(b) => Value::Bool(*b),
        Object::Number(number) => Value::number(number.as_f64()),
        Object::String(string) => Value::String(string.as_bytes().to_vec()),
        Object::Name(name) => Value::Name(name.to_vec()),
        Object::Array(array) => Value::Array(array.raw_iter().map(|item| value(&item)).collect()),
        Object::Dict(dict) => Value::Dict(dict_value(dict)),
        Object::Stream(stream) => Value::Stream(Box::new(stream_value(stream))),
    }
}

/// The dictionary `dict` read, each value held as it is.
pub(crate) fn dict_value(dict: &Dict<'_>) -> Dictionary {
    dict.entries()
        .map(|(key, item)| (key.to_vec(), value(&item)))
        .collect()
}

/// The stream `stream` read: its dictionary, and its data as stored.
pub(crate) fn stream_value(stream: &Source<'_>) -> Stream {
    let mut dict = dict_value(stream.dict());
    dict.remove(b"Length");
    Stream::new(dict, stream.raw_data().into_owned())
}

/// The objects one redaction substitutes for those of a document, and the
/// objects it adds.
#[derive(Debug, Default)]
pub(crate) struct Changes {
    replaced: BTreeMap<u32, Value>,
    added: Vec<Value>,
}

impl Changes {
    /// No change.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Write `value` in place of the object `id`.
    pub(crate) fn replace(&mut self, id: ObjectId, value: impl Into<Value>) {
        self.replaced.insert(id.obj_number, value.into());
    }

    /// What the object `id` is replaced with, to change further.
    pub(crate) fn replacement(&mut self, id: ObjectId) -> Option<&mut Value> {
        self.replaced.get_mut(&id.obj_number)
    }

    /// The object `id` as it is written, to change: its replacement, else
    /// `original` replacing it.
    pub(crate) fn entry(&mut self, id: ObjectId, original: impl FnOnce() -> Value) -> &mut Value {
        self.replaced.entry(id.obj_number).or_insert_with(original)
    }

    /// Add `value` as an object of its own, referred to by the value
    /// returned once the changes are applied.
    pub(crate) fn add(&mut self, value: impl Into<Value>) -> Added {
        self.added.push(value.into());
        Added(self.added.len() - 1)
    }

    /// Whether nothing changes.
    pub(crate) fn is_empty(&self) -> bool {
        self.replaced.is_empty() && self.added.is_empty()
    }
}

/// An object [`Changes`] add, by its place among them; a value referring
/// to it holds [`Added::value`].
#[derive(Debug, Clone, Copy)]
pub(crate) struct Added(usize);

impl Added {
    /// The reference to the added object, numbered past every object the
    /// document already holds. `Value::Ref` of a number the document holds
    /// is otherwise never this large.
    pub(crate) fn value(self) -> Value {
        Value::Ref(Id(ADDED_BASE + self.0 as u32))
    }
}

/// Where the numbers [`Added::value`] refers by begin, renumbered when the
/// changes are applied.
const ADDED_BASE: u32 = 1 << 30;

/// An opened document being redacted: each redaction reads it as it stands
/// and changes it by the objects it substitutes.
#[derive(Debug, Clone)]
pub(crate) struct Edit {
    pdf: Pdf,
}

impl Edit {
    /// An edit of `pdf`.
    pub(crate) fn of(pdf: &Pdf) -> Self {
        Self { pdf: pdf.clone() }
    }

    /// The document as edited so far.
    pub(crate) fn pdf(&self) -> &Pdf {
        &self.pdf
    }

    /// Write `redaction` into the document.
    ///
    /// # Errors
    ///
    /// As `redaction`.
    pub(crate) fn redact<R: Redact>(&mut self, redaction: R) -> Result<()> {
        redaction.redact(self)
    }

    /// Write `changes` into the document: every object its catalog reaches
    /// is copied, as `changes` replaces it, with those `changes` adds.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput)
    /// if the document cannot be read whole or written, or as [`finish`].
    pub(crate) fn apply(&mut self, changes: Changes) -> Result<()> {
        if changes.is_empty() {
            return Ok(());
        }
        let source = &self.pdf;
        let base = next_number(source)
            .filter(|&n| n < ADDED_BASE)
            .ok_or_else(|| Error::malformed("the document holds too many objects to edit"))?;
        let renumber = |value: &mut Value| renumber(value, base);
        let mut out = Document::new(version(source));
        let root = copied(source.file().xref().root_id());
        let mut seen = HashSet::from([root.0]);
        let mut queue = vec![root.0];
        while let Some(number) = queue.pop() {
            let mut value = match changes.replaced.get(&number) {
                Some(value) => value.clone(),
                None => object_value(&held(source, number)?),
            };
            renumber(&mut value);
            references(&value, &mut |id| {
                if id.0 < base && seen.insert(id.0) {
                    queue.push(id.0);
                }
            });
            out.set(Id(number), value);
        }
        for (i, value) in changes.added.into_iter().enumerate() {
            let mut value = value;
            renumber(&mut value);
            references(&value, &mut |id| {
                if id.0 < base && seen.insert(id.0) {
                    queue.push(id.0);
                }
            });
            out.set(Id(base + i as u32), value);
        }
        // An added object may refer to one the document holds that nothing
        // else reached.
        while let Some(number) = queue.pop() {
            let value = object_value(&held(source, number)?);
            references(&value, &mut |id| {
                if id.0 < base && seen.insert(id.0) {
                    queue.push(id.0);
                }
            });
            out.set(Id(number), value);
        }
        out.set_root(root);
        self.pdf = finish(&out)?;
        Ok(())
    }

    /// The document as edited.
    pub(crate) fn finish(self) -> Pdf {
        self.pdf
    }
}

/// The number past every object of `pdf` a reference may name: its
/// dictionaries and streams, which in the normal form are all references
/// name, and its catalog.
fn next_number(pdf: &Pdf) -> Option<u32> {
    let root = pdf.file().xref().root_id().obj_number;
    pdf.file()
        .objects()
        .into_iter()
        .filter_map(|object| match object {
            Object::Dict(dict) => dict.obj_id(),
            Object::Stream(stream) => Some(stream.obj_id()),
            _ => None,
        })
        .map(|id| id.obj_number)
        .chain([root])
        .max()?
        .checked_add(1)
}

/// The object numbered `number` of an edited document, which numbers
/// every object in generation 0, as a rebuild writes it.
fn held(pdf: &Pdf, number: u32) -> Result<Object<'_>> {
    pdf.object(ObjectId::new(number, 0)).ok_or_else(|| {
        Error::malformed(format!("the document refers to a missing object {number}"))
    })
}

/// `value` with each reference to an added object renumbered from `base`.
fn renumber(value: &mut Value, base: u32) {
    match value {
        Value::Ref(id) if id.0 >= ADDED_BASE => id.0 = id.0 - ADDED_BASE + base,
        Value::Array(items) => items.iter_mut().for_each(|item| renumber(item, base)),
        Value::Dict(dict) => dict.iter_mut().for_each(|(_, item)| renumber(item, base)),
        Value::Stream(stream) => stream
            .dict
            .iter_mut()
            .for_each(|(_, item)| renumber(item, base)),
        _ => {}
    }
}

/// Call `found` with each reference `value` holds.
fn references(value: &Value, found: &mut impl FnMut(Id)) {
    match value {
        Value::Ref(id) => found(*id),
        Value::Array(items) => items.iter().for_each(|item| references(item, found)),
        Value::Dict(dict) => dict.iter().for_each(|(_, item)| references(item, found)),
        Value::Stream(stream) => stream
            .dict
            .iter()
            .for_each(|(_, item)| references(item, found)),
        _ => {}
    }
}

/// The version `pdf` declares, as `(major, minor)`.
pub(crate) fn version(pdf: &Pdf) -> (u8, u8) {
    match pdf.file().version() {
        PdfVersion::Pdf10 => (1, 0),
        PdfVersion::Pdf11 => (1, 1),
        PdfVersion::Pdf12 => (1, 2),
        PdfVersion::Pdf13 => (1, 3),
        PdfVersion::Pdf14 => (1, 4),
        PdfVersion::Pdf15 => (1, 5),
        PdfVersion::Pdf16 => (1, 6),
        PdfVersion::Pdf17 => (1, 7),
        PdfVersion::Pdf20 => (2, 0),
    }
}
