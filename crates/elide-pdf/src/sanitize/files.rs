//! Attached files: the attachment tree and the `/AF` associations, each file
//! copied once for each extension its names give it, under a neutral name.
//!
//! A file's name (`/F`, `/UF`, and its key in the attachment tree) is shown in
//! a viewer's attachment panel but never reaches detection, so it is replaced
//! by `attachment-N` with its original extension, which the codec decodes it
//! by. A name that is already neutral stays, so a rebuild of a rebuild renames
//! nothing.

use std::collections::{HashMap, HashSet};

use elide_core::Result;
use elide_core::redaction::HiddenContent;
use lopdf::{Dictionary, Object, ObjectId};

use super::Builder;
use super::content::Fresh;
use super::copy::At;
use super::keys::Kind;
use crate::content::attachment::Tree;
use crate::pdf::reader::{DictionaryExt, DocumentExt, Scope};

/// The neutral name every attached file is given.
const NEUTRAL_PREFIX: &str = "attachment-";

/// The file specification entries the rebuild writes itself.
const SPEC_KEYS: [&[u8]; 5] = [b"Type", b"F", b"UF", b"EF", b"AFRelationship"];

/// The attached files a rebuild copies.
pub(super) struct Files {
    /// The neutral names given.
    names: HashSet<String>,
    /// The neutral names to try.
    fresh: Fresh,
    /// The copy of each embedded file made for a name of each extension.
    copies: HashMap<ObjectId, HashMap<Option<String>, Object>>,
}

impl Default for Files {
    fn default() -> Self {
        Self {
            names: HashSet::new(),
            fresh: Fresh::new(NEUTRAL_PREFIX),
            copies: HashMap::new(),
        }
    }
}

impl Builder<'_> {
    /// The attachment tree rebuilt as one leaf of its files, sorted by their
    /// neutral names, or `None` when the document attaches nothing.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if
    /// the tree cannot be read.
    pub(super) fn attachment_tree(&mut self) -> Result<Option<Dictionary>> {
        let tree = Tree::read(self.source)?;
        // A value embedding no file is a path to one elsewhere, not a file
        // the document carries.
        self.stripped
            .record(HiddenContent::FilePath, None, tree.unembedded);
        let mut pairs = Vec::new();
        for entry in &tree.entries {
            if let Some(spec) =
                self.copy(Kind::FileSpec, entry.value, At::top(&Scope::default()))?
            {
                pairs.push((self.spec_name(&spec), spec));
            }
        }
        if pairs.is_empty() {
            return Ok(None);
        }
        pairs.sort_by(|(a, _), (b, _)| a.cmp(b));
        pairs.dedup();
        let leaf: Vec<Object> = pairs
            .into_iter()
            .flat_map(|(name, spec)| [Object::string_literal(name), spec])
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
            // A file specification given as a string is a path to a file
            // elsewhere, not a file the document embeds.
            if source.resolve_dict(spec).is_none() {
                self.record(HiddenContent::FilePath);
                continue;
            }
            match self.copy(Kind::FileSpec, spec, At::top(&Scope::default()))? {
                Some(spec) => out.push(spec),
                None => self.residue += 1,
            }
        }
        Ok((!out.is_empty()).then_some(Object::Array(out)))
    }

    /// The file specification `spec` copied, its file under a neutral name of
    /// its own and with its own relationship; the file it embeds is copied
    /// once for each extension the specifications naming it give it, so the
    /// file each name says it is is a file of its own, decoded as that.
    pub(super) fn file_spec(&mut self, spec: &Dictionary) -> Result<Option<Object>> {
        let source = self.source;
        let Some((stream, name)) = spec.embedded_file(source) else {
            self.residue += 1;
            return Ok(None);
        };
        for (key, _) in spec.iter() {
            if !SPEC_KEYS.contains(&key.as_slice()) {
                self.drop_key(key);
            }
        }
        let name = name.unwrap_or_default();
        let Some(file) = self.embedded_file(stream, extension(&name))? else {
            return Ok(None);
        };
        let neutral = self.neutral(&name);
        let mut copy = Dictionary::from_iter([
            ("Type", Object::Name(b"Filespec".to_vec())),
            ("F", Object::string_literal(neutral.clone())),
            ("UF", Object::string_literal(neutral)),
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
        Ok(Some(Object::Dictionary(copy)))
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
    ) -> Result<Option<Object>> {
        let copies = self.files.copies.get(&stream);
        if let Some(file) = copies.and_then(|copies| copies.get(&extension)) {
            return Ok(Some(file.clone()));
        }
        let scope = Scope::default();
        let first = copies.is_none_or(HashMap::is_empty);
        let file = if first {
            self.copy(
                Kind::EmbeddedFile,
                &Object::Reference(stream),
                At::top(&scope),
            )?
        } else {
            // The stream itself, not a reference to it, is copied anew.
            let Ok(data @ Object::Stream(_)) = self.source.get_object(stream) else {
                return Ok(None);
            };
            self.copy(Kind::EmbeddedFile, data, At::top(&scope))?
        };
        if let Some(file) = &file {
            self.files
                .copies
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
        let neutral = if is_neutral(name) && !self.files.names.contains(name) {
            name.to_owned()
        } else {
            self.record(HiddenContent::Properties);
            let suffix =
                extension(name).map_or_else(String::new, |extension| format!(".{extension}"));
            let Files { names, fresh, .. } = &mut self.files;
            fresh.name_ending(&suffix, |candidate| names.contains(candidate))
        };
        self.files.names.insert(neutral.clone());
        neutral
    }

    /// The name the copied file specification `spec` gives its file.
    fn spec_name(&self, spec: &Object) -> String {
        let dict = match spec {
            Object::Reference(id) => self.out.get_dictionary(*id).ok(),
            Object::Dictionary(dict) => Some(dict),
            _ => None,
        };
        dict.and_then(|dict| dict.get(b"F").ok())
            .and_then(|name| name.as_str().ok())
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
