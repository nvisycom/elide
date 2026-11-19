//! Attached files: the attachment tree and the `/AF` associations, each file
//! copied once for each extension its names give it, under a neutral name.
//!
//! A file's name (`/F`, `/UF`, and its key in the attachment tree) is shown in
//! a viewer's attachment panel but never reaches detection, so it is replaced
//! by `attachment-N` with its original extension, which the codec decodes it
//! by. A name that is already neutral stays, so a rebuild of a rebuild renames
//! nothing.

use std::collections::HashMap;

use elide_core::Result;
use elide_core::redaction::HiddenContent;
use hayro_syntax::object::{Dict, MaybeRef, Object};

use super::copy::At;
use super::keys::Kind;
use super::{Builder, Src, record};
use crate::content::attachment::Tree;
use crate::pdf::object::{Dictionary, Value};
use crate::pdf::reader::{DictExt, ObjectId, Scope, reference_to};

/// The neutral name every attached file is given.
const NEUTRAL_PREFIX: &str = "attachment-";

/// The file specification entries the rebuild writes itself.
const SPEC_KEYS: [&[u8]; 5] = [b"Type", b"F", b"UF", b"EF", b"AFRelationship"];

impl<'a> Builder<'a> {
    /// The attachment tree rebuilt as one leaf of its files, sorted by their
    /// neutral names, or `None` when the document attaches nothing.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if
    /// the tree cannot be read.
    pub(super) fn attachment_tree(&mut self) -> Result<Option<Dictionary>> {
        let tree = Tree::read(self.source)?;
        let mut pairs = Vec::new();
        for entry in &tree.entries {
            let spec = match entry.spec_id {
                Some(spec) => reference_to(spec),
                None => MaybeRef::NotRef(Object::Dict(entry.spec.clone())),
            };
            if let Some(spec) = self.copy(Kind::FileSpec, &spec, At::top(&Scope::default()))? {
                pairs.push((self.spec_name(&spec), spec));
            }
        }
        if pairs.is_empty() {
            return Ok(None);
        }
        pairs.sort_by(|(a, _), (b, _)| a.cmp(b));
        pairs.dedup();
        let leaf: Vec<Value> = pairs
            .into_iter()
            .flat_map(|(name, spec)| [Value::string(name), spec])
            .collect();
        Ok(Some(Dictionary::from_iter([("Names", Value::Array(leaf))])))
    }

    /// The `/AF` value `af`, each of its file specifications copied, or
    /// `None` when none is kept.
    pub(super) fn associated(&mut self, af: &Src<'a>) -> Result<Option<Value>> {
        let source = self.source;
        let Some(specs) = source.resolve(af).and_then(Object::into_array) else {
            self.residue += 1;
            return Ok(None);
        };
        let mut out = Vec::new();
        for spec in specs.raw_iter() {
            // A file specification given as a string is a path to a file
            // elsewhere, not a file the document embeds.
            if !matches!(source.resolve(&spec), Some(Object::Dict(_))) {
                self.record(HiddenContent::FilePath);
                continue;
            }
            match self.copy(Kind::FileSpec, &spec, At::top(&Scope::default()))? {
                Some(spec) => out.push(spec),
                None => self.residue += 1,
            }
        }
        Ok((!out.is_empty()).then_some(Value::Array(out)))
    }

    /// The file specification `spec` copied, its file under a neutral name of
    /// its own and with its own relationship; the file it embeds is copied
    /// once for each extension the specifications naming it give it, so the
    /// file each name says it is is a file of its own, decoded as that.
    pub(super) fn file_spec(&mut self, spec: &Dict<'a>) -> Result<Option<Value>> {
        let Some((stream, name)) = spec.embedded_file() else {
            self.residue += 1;
            return Ok(None);
        };
        for key in spec.keys() {
            if !SPEC_KEYS.contains(&&*key) {
                self.drop_key(&key, record::entry);
            }
        }
        let name = name.unwrap_or_default();
        let Some(file) = self.embedded_file(stream.obj_id(), extension(&name))? else {
            return Ok(None);
        };
        let neutral = self.neutral(&name);
        let mut copy = Dictionary::from_iter([
            ("Type", Value::name("Filespec")),
            ("F", Value::string(&neutral)),
            ("UF", Value::string(&neutral)),
            ("EF", Value::Dict(Dictionary::from_iter([("F", file)]))),
        ]);
        if let Some(relationship) = spec.name(b"AFRelationship") {
            copy.set("AFRelationship", Value::name(&*relationship));
        }
        Ok(Some(Value::Dict(copy)))
    }

    /// The embedded file `stream` copied for a name of `extension`: the copy
    /// already made for it, else the first copy of the stream, else a copy of
    /// its own.
    ///
    /// # Errors
    ///
    /// As [`copy`](Self::copy).
    fn embedded_file(
        &mut self,
        stream: ObjectId,
        extension: Option<String>,
    ) -> Result<Option<Value>> {
        let copies = self.file_copies.get(&stream);
        if let Some(file) = copies.and_then(|copies| copies.get(&extension)) {
            return Ok(Some(file.clone()));
        }
        let scope = Scope::default();
        let first = copies.is_none_or(HashMap::is_empty);
        let file = if first {
            self.copy(Kind::EmbeddedFile, &reference_to(stream), At::top(&scope))?
        } else {
            // The stream itself, not a reference to it, is copied anew.
            let Some(data @ Object::Stream(_)) = self.source.object(stream) else {
                return Ok(None);
            };
            self.copy(Kind::EmbeddedFile, &MaybeRef::NotRef(data), At::top(&scope))?
        };
        if let Some(file) = &file {
            self.file_copies
                .entry(stream)
                .or_default()
                .insert(extension, file.clone());
        }
        Ok(file)
    }

    /// The neutral name a file named `name` is given: `name` itself when it
    /// is neutral and not yet given, else the next neutral name not given,
    /// recorded as a property removed.
    fn neutral(&mut self, name: &str) -> String {
        let neutral = if is_neutral(name) && !self.file_names.contains(name) {
            name.to_owned()
        } else {
            self.record(HiddenContent::Properties);
            loop {
                let candidate = neutral_name(self.next_file, name);
                self.next_file += 1;
                if !self.file_names.contains(&candidate) {
                    break candidate;
                }
            }
        };
        self.file_names.insert(neutral.clone());
        neutral
    }

    /// The name the copied file specification `spec` gives its file.
    fn spec_name(&self, spec: &Value) -> String {
        let dict = match spec {
            Value::Ref(id) => self.out.get(*id).and_then(Value::as_dict),
            Value::Dict(dict) => Some(dict),
            _ => None,
        };
        dict.and_then(|dict| dict.get(b"F"))
            .and_then(Value::as_string)
            .map(|name| String::from_utf8_lossy(name).into_owned())
            .unwrap_or_default()
    }
}

/// Whether `name` is already neutral: exactly what [`neutral_name`] writes
/// for its number, so nothing of an original name survives after it.
fn is_neutral(name: &str) -> bool {
    let stem = name.split_once('.').map_or(name, |(stem, _)| stem);
    stem.strip_prefix(NEUTRAL_PREFIX)
        .filter(|number| number.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|number| number.parse().ok())
        .is_some_and(|index| neutral_name(index, name) == name)
}

/// The neutral name of the `index`th attached file, keeping the extension of
/// its `original` name when it has a plain one.
fn neutral_name(index: usize, original: &str) -> String {
    match extension(original) {
        Some(extension) => format!("{NEUTRAL_PREFIX}{index}.{extension}"),
        None => format!("{NEUTRAL_PREFIX}{index}"),
    }
}

/// The plain extension of the file name `name`, lowercased: what the codec
/// decodes the file by.
fn extension(name: &str) -> Option<String> {
    name.rsplit_once('.')
        .map(|(_, extension)| extension)
        .filter(|extension| {
            (1..=8).contains(&extension.len())
                && extension.bytes().all(|b| b.is_ascii_alphanumeric())
        })
        .map(str::to_ascii_lowercase)
}
