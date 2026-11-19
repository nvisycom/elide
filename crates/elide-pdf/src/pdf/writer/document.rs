//! A document being written: its objects, numbered as they are added, and
//! its catalog, saved through pdf-writer.

use std::collections::{BTreeMap, HashSet};

use elide_core::{Error, Result};
use pdf_writer::{Chunk, Name, Obj, Ref, Str};

use crate::pdf::object::{Dictionary, Id, Stream, Value};

/// A document being written.
#[derive(Debug, Clone)]
pub(crate) struct Document {
    /// The version its header declares, as `(major, minor)`.
    version: (u8, u8),
    objects: BTreeMap<Id, Value>,
    /// The number the next object takes.
    next: u32,
    /// The catalog, which the trailer's `/Root` names.
    root: Option<Id>,
    /// The document information dictionary, which the trailer's `/Info`
    /// names.
    info: Option<Id>,
    /// Whether every object is written, those nothing refers to too, as a
    /// document carrying a prior revision's objects does.
    #[cfg(test)]
    all: bool,
}

impl Document {
    /// An empty document of `version`.
    pub(crate) fn new(version: (u8, u8)) -> Self {
        Self {
            version,
            objects: BTreeMap::new(),
            next: 1,
            root: None,
            info: None,
            #[cfg(test)]
            all: false,
        }
    }

    /// A number for an object written later, with [`set`](Self::set).
    pub(crate) fn reserve(&mut self) -> Id {
        let id = Id(self.next);
        self.next += 1;
        id
    }

    /// Add `value` as an object of its own, returning its number.
    pub(crate) fn add(&mut self, value: impl Into<Value>) -> Id {
        let id = self.reserve();
        self.objects.insert(id, value.into());
        id
    }

    /// Write `value` as the object `id`.
    pub(crate) fn set(&mut self, id: Id, value: impl Into<Value>) {
        self.objects.insert(id, value.into());
    }

    /// The object `id`.
    pub(crate) fn get(&self, id: Id) -> Option<&Value> {
        self.objects.get(&id)
    }

    /// The object `id`, mutably.
    pub(crate) fn get_mut(&mut self, id: Id) -> Option<&mut Value> {
        self.objects.get_mut(&id)
    }

    /// Make the object `id` the catalog.
    pub(crate) fn set_root(&mut self, id: Id) {
        self.root = Some(id);
    }

    /// Make the object `id` the document information dictionary.
    #[cfg(any(test, feature = "fixtures"))]
    pub(crate) fn set_info(&mut self, id: Id) {
        self.info = Some(id);
    }

    /// Each object and its number.
    #[cfg(test)]
    pub(crate) fn objects(&self) -> impl Iterator<Item = (Id, &Value)> {
        self.objects.iter().map(|(&id, value)| (id, value))
    }

    /// Declare `version` in the header.
    #[cfg(test)]
    pub(crate) fn set_version(&mut self, version: (u8, u8)) {
        self.version = version;
    }

    /// Write every object, those nothing refers to too.
    #[cfg(test)]
    pub(crate) fn keep_all(&mut self) {
        self.all = true;
    }

    /// The catalog's number, once set.
    #[cfg(any(test, feature = "fixtures"))]
    pub(crate) fn root(&self) -> Option<Id> {
        self.root
    }

    /// The document saved: every object its catalog reaches, numbered as
    /// added.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput)
    /// if it has no catalog, a stream is held in place rather than as an
    /// object, or a number is not finite.
    pub(crate) fn save(&self) -> Result<Vec<u8>> {
        let root = self
            .root
            .ok_or_else(|| Error::malformed("the document written has no catalog"))?;
        let mut pdf = pdf_writer::Pdf::new();
        pdf.set_version(self.version.0, self.version.1);
        for id in self.reachable(root) {
            let value = &self.objects[&id];
            if Some(id) == self.info {
                let Value::Dict(info) = value else {
                    return Err(Error::malformed(
                        "the document information written is not a dictionary",
                    ));
                };
                let mut writer = pdf.document_info(reference(id)?);
                for (key, value) in info.iter() {
                    write(writer.insert(Name(key)), value)?;
                }
                continue;
            }
            if id == root {
                let Value::Dict(catalog) = value else {
                    return Err(Error::malformed("the catalog written is not a dictionary"));
                };
                // The catalog writer writes its `/Type` itself.
                let mut writer = pdf.catalog(reference(id)?);
                for (key, value) in catalog.iter().filter(|(key, _)| *key != b"Type") {
                    write(writer.insert(Name(key)), value)?;
                }
                continue;
            }
            write_object(&mut pdf, id, value)?;
        }
        Ok(pdf.finish())
    }

    /// The objects `root`, and the information dictionary, reach, in
    /// number order.
    fn reachable(&self, root: Id) -> Vec<Id> {
        #[cfg(test)]
        if self.all {
            return self.objects.keys().copied().collect();
        }
        let mut seen: HashSet<Id> = [Some(root), self.info].into_iter().flatten().collect();
        let mut stack: Vec<Id> = seen.iter().copied().collect();
        while let Some(id) = stack.pop() {
            // A reference to an object not held is written as it is: a
            // reader reads it as null (7.3.10).
            let Some(value) = self.objects.get(&id) else {
                continue;
            };
            let mut values = vec![value];
            while let Some(value) = values.pop() {
                match value {
                    Value::Ref(id) => {
                        if seen.insert(*id) {
                            stack.push(*id);
                        }
                    }
                    Value::Array(items) => values.extend(items),
                    Value::Dict(dict) => values.extend(dict.iter().map(|(_, v)| v)),
                    Value::Stream(stream) => values.extend(stream.dict.iter().map(|(_, v)| v)),
                    _ => {}
                }
            }
        }
        let mut ids: Vec<Id> = seen
            .into_iter()
            .filter(|id| self.objects.contains_key(id))
            .collect();
        ids.sort_unstable();
        ids
    }
}

/// Write the object `id` holding `value` into `chunk`.
fn write_object(chunk: &mut Chunk, id: Id, value: &Value) -> Result<()> {
    let id = reference(id)?;
    match value {
        Value::Stream(stream) => {
            let Stream { dict, data } = stream.as_ref();
            let mut writer = chunk.stream(id, data);
            // pdf-writer writes `/Length` from the data.
            for (key, value) in dict.iter().filter(|(key, _)| *key != b"Length") {
                write(writer.insert(Name(key)), value)?;
            }
            Ok(())
        }
        value => write(chunk.indirect(id), value),
    }
}

/// Write `value` held in place: into an object, or as a content operand.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if
/// it is or holds a stream, which is only ever an object of its own, or a
/// number that is not finite.
pub(crate) fn write(obj: Obj<'_>, value: &Value) -> Result<()> {
    match value {
        Value::Null => obj.primitive(pdf_writer::Null),
        Value::Bool(b) => obj.primitive(*b),
        // An integer past what a PDF integer holds is written as a real.
        Value::Int(i) => match i32::try_from(*i) {
            Ok(i) => obj.primitive(i),
            Err(_) => obj.primitive(*i as f32),
        },
        Value::Real(r) => {
            let real = *r as f32;
            if !real.is_finite() {
                return Err(Error::malformed("a number written is not finite"));
            }
            obj.primitive(real);
        }
        Value::Name(name) => obj.primitive(Name(name)),
        Value::String(bytes) => obj.primitive(Str(bytes)),
        Value::Array(items) => {
            let mut array = obj.array();
            for item in items {
                write(array.push(), item)?;
            }
        }
        Value::Dict(dict) => write_dict(obj, dict)?,
        Value::Stream(_) => {
            return Err(Error::malformed(
                "a stream written is held in place, not as an object",
            ));
        }
        Value::Ref(id) => obj.primitive(reference(*id)?),
    }
    Ok(())
}

/// Write `dict` held in place.
fn write_dict(obj: Obj<'_>, dict: &Dictionary) -> Result<()> {
    let mut writer = obj.dict();
    for (key, value) in dict.iter() {
        write(writer.insert(Name(key)), value)?;
    }
    Ok(())
}

/// The pdf-writer reference to the object `id`.
fn reference(id: Id) -> Result<Ref> {
    i32::try_from(id.0)
        .ok()
        .filter(|&n| n > 0)
        .map(Ref::new)
        .ok_or_else(|| Error::malformed(format!("object number {} cannot be written", id.0)))
}
