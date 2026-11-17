//! Attached files: the attachment tree and the `/AF` associations, each file
//! copied once under a neutral name.
//!
//! A file's name (`/F`, `/UF`, and its key in the attachment tree) is shown in
//! a viewer's attachment panel but never reaches detection, so it is replaced
//! by `attachment-N` with its original extension, which the codec decodes it
//! by. A name that is already neutral stays, so a rebuild of a rebuild renames
//! nothing.

use elide_core::Result;
use elide_core::redaction::HiddenContent;
use lopdf::{Dictionary, Object, ObjectId};

use super::keys::Kind;
use super::{Rebuild, record};
use crate::document::attachment::tree::Tree;
use crate::document::object::{DictionaryExt, DocumentExt};
use crate::document::scope::Scope;

/// The neutral name every attached file is given.
const NEUTRAL_PREFIX: &str = "attachment-";

/// The file specification entries the rebuild writes itself.
const SPEC_KEYS: [&[u8]; 5] = [b"Type", b"F", b"UF", b"EF", b"AFRelationship"];

impl Rebuild<'_> {
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
            if let Some((spec, name)) = self.attach(entry.stream, &entry.name, entry.spec)? {
                pairs.push((name, spec));
            }
        }
        if pairs.is_empty() {
            return Ok(None);
        }
        pairs.sort();
        pairs.dedup();
        let leaf: Vec<Object> = pairs
            .into_iter()
            .flat_map(|(name, spec)| [Object::string_literal(name), Object::Reference(spec)])
            .collect();
        Ok(Some(Dictionary::from_iter([(
            "Names",
            Object::Array(leaf),
        )])))
    }

    /// The `/AF` value `af`, each of its file specifications copied, or
    /// `None` when none is kept.
    pub(super) fn associated(&mut self, af: &Object) -> Result<Option<Object>> {
        let source = self.source;
        let Some(specs) = source.resolve_array(af) else {
            self.residue += 1;
            return Ok(None);
        };
        let mut out = Vec::new();
        for spec in specs {
            match self.copy(Kind::FileSpec, spec, &Scope::default(), 0)? {
                Some(spec) => out.push(spec),
                None => self.residue += 1,
            }
        }
        Ok((!out.is_empty()).then_some(Object::Array(out)))
    }

    /// The file specification `spec` copied, its file under a neutral name.
    pub(super) fn file_spec(&mut self, spec: &Dictionary) -> Result<Option<Object>> {
        let source = self.source;
        let stream = spec
            .get(b"EF")
            .ok()
            .and_then(|files| source.resolve_dict(files))
            .and_then(|files| files.get(b"UF").or_else(|_| files.get(b"F")).ok())
            .and_then(|file| file.as_reference().ok());
        let Some(stream) = stream else {
            self.residue += 1;
            return Ok(None);
        };
        let name = spec.file_name().unwrap_or_default();
        Ok(self
            .attach(stream, &name, spec)?
            .map(|(spec, _)| Object::Reference(spec)))
    }

    /// The copy of the file `stream`, named `name` by the file specification
    /// `spec`: its file specification and neutral name, copied once.
    fn attach(
        &mut self,
        stream: ObjectId,
        name: &str,
        spec: &Dictionary,
    ) -> Result<Option<(ObjectId, String)>> {
        if let Some(copy) = self.files.get(&stream) {
            return Ok(Some(copy.clone()));
        }
        for (key, _) in spec.iter() {
            if !SPEC_KEYS.contains(&key.as_slice()) {
                self.drop_key(key, record::entry);
            }
        }
        let Some(file) = self.copy(
            Kind::EmbeddedFile,
            &Object::Reference(stream),
            &Scope::default(),
            0,
        )?
        else {
            return Ok(None);
        };
        let neutral = if is_neutral(name) {
            name.to_owned()
        } else {
            self.record(HiddenContent::Properties);
            let taken = |candidate: &String| self.files.values().any(|(_, name)| name == candidate);
            (self.files.len() + 1..)
                .map(|index| neutral_name(index, name))
                .find(|candidate| !taken(candidate))
                .unwrap_or_default()
        };
        let mut copy = Dictionary::from_iter([
            ("Type", Object::Name(b"Filespec".to_vec())),
            ("F", Object::string_literal(neutral.clone())),
            ("UF", Object::string_literal(neutral.clone())),
            (
                "EF",
                Object::Dictionary(Dictionary::from_iter([("F", file)])),
            ),
        ]);
        if let Some(relationship) = spec
            .get(b"AFRelationship")
            .ok()
            .and_then(|r| self.source.resolve(r))
            .filter(|r| r.as_name().is_ok())
        {
            copy.set("AFRelationship", relationship.clone());
        }
        let copy = self.out.add_object(copy);
        self.files.insert(stream, (copy, neutral.clone()));
        Ok(Some((copy, neutral)))
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
    let extension = original
        .rsplit_once('.')
        .map(|(_, extension)| extension)
        .filter(|extension| {
            (1..=8).contains(&extension.len())
                && extension.bytes().all(|b| b.is_ascii_alphanumeric())
        });
    match extension {
        Some(extension) => format!("{NEUTRAL_PREFIX}{index}.{}", extension.to_ascii_lowercase()),
        None => format!("{NEUTRAL_PREFIX}{index}"),
    }
}
