//! Copying an object as its [`Kind`]: only the keys the kind keeps, every
//! value resolved, every child copied as the kind its key holds.

use elide_core::redaction::HiddenContent;
use elide_core::{Error, Result};
use lopdf::{Dictionary, Object, ObjectId, Stream};

use super::keys::{self, FONT, FORM, Kind, SHADING_PATTERN, STREAM, TILING, TO_UNICODE};
use super::{MAX_NESTING, Rebuild, ScopedCopy, record};
use crate::document::image;
use crate::document::object::{DocumentExt, ObjectExt, StreamExt};
use crate::text::{readable_cmap, to_unicode};

/// How many references a value is followed through.
const MAX_HOPS: usize = 16;

impl<'a> Rebuild<'a> {
    /// `value` copied as `kind`, its names resolved in `scope` where it is
    /// content; `None` when nothing of it is kept.
    pub(super) fn copy<'s>(
        &mut self,
        kind: Kind,
        value: &'s Object,
        scope: &[&'s Dictionary],
        depth: usize,
    ) -> Result<Option<Object>>
    where
        'a: 's,
    {
        if depth > MAX_NESTING {
            return Err(Error::resource_limit(format!(
                "objects nest deeper than {MAX_NESTING} levels"
            )));
        }
        match value {
            Object::Reference(id) => self.copy_reference(kind, *id, scope, depth),
            Object::Dictionary(_) | Object::Stream(_) => {
                self.copy_object(kind, None, value, scope, depth)
            }
            Object::Array(items) if kind == Kind::ColorSpace => {
                self.color_space(items, scope, depth)
            }
            Object::Array(items) => {
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    match self.copy(kind, item, scope, depth + 1)? {
                        Some(item) => out.push(item),
                        None => return Ok(None),
                    }
                }
                Ok(Some(Object::Array(out)))
            }
            Object::Null
            | Object::Boolean(_)
            | Object::Integer(_)
            | Object::Real(_)
            | Object::Name(_)
            | Object::String(..) => Ok(Some(value.clone())),
        }
    }

    /// The object `id` refers to copied as `kind`: a dictionary or stream as
    /// its own object, copied once per kind (and per scope, for content that
    /// borrows the scope it is drawn in); anything else in place.
    fn copy_reference<'s>(
        &mut self,
        kind: Kind,
        id: ObjectId,
        scope: &[&'s Dictionary],
        depth: usize,
    ) -> Result<Option<Object>>
    where
        'a: 's,
    {
        let source = self.source;
        let mut target = id;
        let mut hops = 0;
        let object = loop {
            let Ok(object) = source.get_object(target) else {
                self.residue += 1;
                return Ok(None);
            };
            match object {
                Object::Reference(next) if hops < MAX_HOPS => {
                    target = *next;
                    hops += 1;
                }
                Object::Reference(_) => {
                    self.residue += 1;
                    return Ok(None);
                }
                object => break object,
            }
        };
        if !matches!(object, Object::Dictionary(_) | Object::Stream(_)) {
            return self.copy(kind, object, scope, depth + 1);
        }
        // Content that resolves its names in the scope it is drawn in is
        // copied once for each scope whose resources differ; any other
        // object is copied once.
        let drawn_in: &[&Dictionary] = if self.borrows_scope(kind, object) {
            scope
        } else {
            &[]
        };
        let scopes = &self.scopes;
        let same_scope =
            |(index, _): &&ScopedCopy| scopes[*index].iter().eq(drawn_in.iter().copied());
        if let Some((_, copy)) = self
            .copies
            .get(&(target, kind))
            .and_then(|copies| copies.iter().find(same_scope))
        {
            return Ok(Some(Object::Reference(*copy)));
        }
        // The copy's id is taken first, so an object that refers back to
        // itself (a form drawing a pattern drawing the form) refers to it.
        let index = self.scope_index(drawn_in);
        let copy = self.out.new_object_id();
        self.copies
            .entry((target, kind))
            .or_default()
            .push((index, copy));
        match self.copy_object(kind, Some(target), object, scope, depth + 1)? {
            Some(object) => {
                self.out.objects.insert(copy, object);
                Ok(Some(Object::Reference(copy)))
            }
            None => {
                if let Some(copies) = self.copies.get_mut(&(target, kind)) {
                    copies.retain(|(_, id)| *id != copy);
                }
                Ok(None)
            }
        }
    }

    /// The index in [`Rebuild::scopes`] of the scope `resources`, kept there
    /// the first time it is seen. It is looked up only when a copy is made,
    /// so the scan is paid once per copy, not once per reference.
    fn scope_index(&mut self, resources: &[&Dictionary]) -> usize {
        let same = |scope: &Vec<Dictionary>| scope.iter().eq(resources.iter().copied());
        self.scopes.iter().position(same).unwrap_or_else(|| {
            self.scopes
                .push(resources.iter().map(|dict| (*dict).clone()).collect());
            self.scopes.len() - 1
        })
    }

    /// Whether `object`, copied as `kind`, is content that resolves its names
    /// in the scope it is drawn in: a form, tiling pattern, or Type3 font
    /// whose `/Resources`, if any, is not a dictionary it can use.
    fn borrows_scope(&self, kind: Kind, object: &Object) -> bool {
        let dict = match object {
            Object::Dictionary(dict) => dict,
            Object::Stream(stream) => &stream.dict,
            _ => return false,
        };
        let content = match kind {
            Kind::Form | Kind::Pattern => matches!(object, Object::Stream(_)),
            Kind::Font => self.name(dict, b"Subtype") == Some(b"Type3"),
            _ => false,
        };
        content
            && dict
                .get(b"Resources")
                .ok()
                .and_then(|resources| self.source.resolve_dict(resources))
                .is_none()
    }

    /// The dictionary or stream `value` (the source object `id`, when it is
    /// one) copied as `kind`.
    fn copy_object<'s>(
        &mut self,
        kind: Kind,
        id: Option<ObjectId>,
        value: &'s Object,
        scope: &[&'s Dictionary],
        depth: usize,
    ) -> Result<Option<Object>>
    where
        'a: 's,
    {
        match (kind, value) {
            (Kind::Font, Object::Dictionary(dict)) => self.font(dict, scope, depth).map(Some),
            (Kind::Form, Object::Stream(stream)) => self.form(stream, FORM, scope, depth).map(Some),
            (Kind::Pattern, _) => self.pattern(value, scope, depth),
            (Kind::FileSpec, Object::Dictionary(dict)) => self.file_spec(dict),
            (Kind::ToUnicode, Object::Stream(stream)) if !self.external(&stream.dict) => {
                self.unicode_map(value, stream, scope, depth)
            }
            (Kind::Image | Kind::Mask, Object::Stream(stream))
                if !image::is_surfaceable(self.source, stream) =>
            {
                self.unreadable(id);
                Ok(None)
            }
            (Kind::Map(inner), Object::Dictionary(dict)) => {
                let mut out = Dictionary::new();
                for (key, value) in dict.iter() {
                    if let Some(value) = self.copy(*inner, value, scope, depth + 1)? {
                        out.set(key.clone(), value);
                    }
                }
                Ok(Some(Object::Dictionary(out)))
            }
            _ => match keys::fields(kind) {
                Some(fields) => self.table(value, fields, scope, depth),
                None => {
                    self.residue += 1;
                    Ok(None)
                }
            },
        }
    }

    /// Record an image no codec reads as removed.
    pub(super) fn unreadable(&mut self, id: Option<ObjectId>) {
        self.record(HiddenContent::Unreadable);
        if let Some(id) = id {
            self.cut.insert(id);
        }
    }

    /// The dictionary or stream `value` copied keeping `fields`, a stream's
    /// data as it is stored.
    fn table<'s>(
        &mut self,
        value: &'s Object,
        fields: &[(&[u8], Kind)],
        scope: &[&'s Dictionary],
        depth: usize,
    ) -> Result<Option<Object>>
    where
        'a: 's,
    {
        Ok(Some(match value {
            Object::Dictionary(dict) => {
                Object::Dictionary(self.keys(dict, fields, &[], scope, depth)?)
            }
            Object::Stream(stream) if self.external(&stream.dict) => {
                self.record(HiddenContent::Unreadable);
                return Ok(None);
            }
            Object::Stream(stream) => {
                let dict = self.stream_keys(&stream.dict, fields, &[], scope, depth)?;
                Object::Stream(Stream::new(dict, stream.content.clone()).with_compression(false))
            }
            _ => return Ok(None),
        }))
    }

    /// Whether the stream `dict` describes holds data the rebuild cannot
    /// carry: data in another file (`/F`), or under a `/Crypt` filter.
    fn external(&self, dict: &Dictionary) -> bool {
        let source = self.source;
        let crypt = dict
            .get(b"Filter")
            .ok()
            .and_then(|filter| source.resolve(filter))
            .is_some_and(|filter| match filter {
                Object::Name(name) => name == b"Crypt",
                Object::Array(names) => names.iter().any(|name| {
                    source.resolve(name).and_then(|n| n.as_name().ok()) == Some(b"Crypt")
                }),
                _ => false,
            });
        crypt || dict.get(b"F").is_ok()
    }

    /// The entries of `dict` that `fields` keeps, each copied as its kind;
    /// every other entry but those in `handled` is recorded as dropped.
    pub(super) fn keys<'s>(
        &mut self,
        dict: &'s Dictionary,
        fields: &[(&[u8], Kind)],
        handled: &[&[u8]],
        scope: &[&'s Dictionary],
        depth: usize,
    ) -> Result<Dictionary>
    where
        'a: 's,
    {
        let mut out = Dictionary::new();
        for (key, value) in dict.iter() {
            if handled.contains(&key.as_slice()) {
                continue;
            }
            match fields.iter().find(|(field, _)| *field == key.as_slice()) {
                Some(&(_, kind)) => {
                    if let Some(value) = self.copy(kind, value, scope, depth + 1)? {
                        out.set(key.clone(), value);
                    }
                }
                None => self.drop_key(key, record::entry),
            }
        }
        Ok(out)
    }

    /// As [`keys`](Self::keys), for a stream's dictionary: the stream encoding
    /// entries are kept too, and its length is rewritten on save.
    fn stream_keys<'s>(
        &mut self,
        dict: &'s Dictionary,
        fields: &[(&[u8], Kind)],
        handled: &[&[u8]],
        scope: &[&'s Dictionary],
        depth: usize,
    ) -> Result<Dictionary>
    where
        'a: 's,
    {
        let fields: Vec<(&[u8], Kind)> = fields.iter().chain(STREAM).copied().collect();
        let handled: Vec<&[u8]> = handled.iter().copied().chain([&b"Length"[..]]).collect();
        self.keys(dict, &fields, &handled, scope, depth)
    }

    /// A font, a Type3 font's glyph procedures rebuilt as content with names
    /// resolved in its own resources, else in `scope`.
    fn font<'s>(
        &mut self,
        dict: &'s Dictionary,
        scope: &[&'s Dictionary],
        depth: usize,
    ) -> Result<Object>
    where
        'a: 's,
    {
        let source = self.source;
        let subtype = self.name(dict, b"Subtype");
        let handled: &[&[u8]] = match subtype {
            Some(b"Type3") => &[b"CharProcs", b"Resources"],
            Some(b"Type0") => &[b"Encoding"],
            _ => &[],
        };
        let mut out = self.keys(dict, FONT, handled, scope, depth)?;
        if subtype == Some(b"Type0") {
            if let Ok(encoding) = dict.get(b"Encoding")
                && let Some(encoding) = self.copy(Kind::CMap, encoding, scope, depth + 1)?
            {
                out.set("Encoding", encoding);
            }
            return Ok(Object::Dictionary(out));
        }
        if subtype != Some(b"Type3") {
            return Ok(Object::Dictionary(out));
        }
        let own = dict
            .get(b"Resources")
            .ok()
            .and_then(|r| source.resolve_dict(r));
        let inner: Vec<&Dictionary> = match own {
            Some(own) => vec![own],
            None => scope.to_vec(),
        };
        let mut resources = Dictionary::new();
        let mut procs = Dictionary::new();
        let glyphs = dict
            .get(b"CharProcs")
            .ok()
            .and_then(|p| source.resolve_dict(p));
        for (name, glyph) in glyphs.into_iter().flat_map(Dictionary::iter) {
            let Some(Object::Stream(glyph)) = source.resolve(glyph) else {
                self.residue += 1;
                continue;
            };
            let (content, used) = self.content(&glyph.decoded()?, &inner, depth + 1)?;
            merge(&mut resources, used);
            procs.set(name.clone(), self.out.add_object(compressed(content)?));
        }
        out.set("CharProcs", procs);
        out.set("Resources", resources);
        Ok(Object::Dictionary(out))
    }

    /// A form XObject or tiling pattern `stream` keeping `fields`, its content
    /// rebuilt with names resolved in its own resources, else in `scope`.
    fn form<'s>(
        &mut self,
        stream: &'s Stream,
        fields: &[(&[u8], Kind)],
        scope: &[&'s Dictionary],
        depth: usize,
    ) -> Result<Object>
    where
        'a: 's,
    {
        let source = self.source;
        let own = stream
            .dict
            .get(b"Resources")
            .ok()
            .and_then(|r| source.resolve_dict(r));
        let inner: Vec<&Dictionary> = match own {
            Some(own) => vec![own],
            None => scope.to_vec(),
        };
        let (content, resources) = self.content(&stream.decoded()?, &inner, depth + 1)?;
        let mut dict = self.stream_keys(
            &stream.dict,
            fields,
            &[b"Resources", b"Filter", b"DecodeParms", b"DL"],
            scope,
            depth,
        )?;
        dict.set("Resources", resources);
        let mut out = compressed(content)?;
        for (key, value) in dict.iter() {
            out.dict.set(key.clone(), value.clone());
        }
        Ok(Object::Stream(out))
    }

    /// A `/ToUnicode` CMap `stream` (the object `value`), its destinations
    /// written as the text detection reads; as it is stored when none
    /// changes, and dropped as unreadable when lopdf, which detection decodes
    /// text with, can't read it, so a viewer reads no text detection did not.
    ///
    /// # Errors
    ///
    /// As [`to_unicode`] and [`readable_cmap`]: the rewrites across the
    /// document add at most
    /// [`MAX_CONTENT_BYTES`](crate::document::object::MAX_CONTENT_BYTES).
    fn unicode_map<'s>(
        &mut self,
        value: &'s Object,
        stream: &'s Stream,
        scope: &[&'s Dictionary],
        depth: usize,
    ) -> Result<Option<Object>>
    where
        'a: 's,
    {
        if to_unicode(self.source, stream)?.is_none() {
            self.record(HiddenContent::Unreadable);
            return Ok(None);
        }
        let content = match stream.decoded() {
            Ok(data) => readable_cmap(&data, &mut self.cmap_budget)?,
            Err(_) => None,
        };
        let Some(content) = content else {
            return self.table(value, TO_UNICODE, scope, depth);
        };
        let dict = self.stream_keys(
            &stream.dict,
            TO_UNICODE,
            &[b"Filter", b"DecodeParms", b"DL"],
            scope,
            depth,
        )?;
        let mut out = compressed(content)?;
        for (key, value) in dict.iter() {
            out.dict.set(key.clone(), value.clone());
        }
        Ok(Some(Object::Stream(out)))
    }

    /// A pattern: a tiling one rebuilt as content, a shading one copied.
    fn pattern<'s>(
        &mut self,
        value: &'s Object,
        scope: &[&'s Dictionary],
        depth: usize,
    ) -> Result<Option<Object>>
    where
        'a: 's,
    {
        // A pattern's kind is its shape: a stream is a tiling pattern, a
        // dictionary a shading pattern. `/PatternType` is written to match,
        // so a viewer and the text walk read the copy as the same kind.
        let (mut copy, kind) = match value {
            Object::Stream(stream) => (self.form(stream, TILING, scope, depth)?, 1),
            Object::Dictionary(_) => match self.table(value, SHADING_PATTERN, scope, depth)? {
                Some(copy) => (copy, 2),
                None => return Ok(None),
            },
            _ => {
                self.residue += 1;
                return Ok(None);
            }
        };
        if let Some(dict) = copy.dict_mut() {
            dict.set("PatternType", Object::Integer(kind));
        }
        Ok(Some(copy))
    }

    /// A colour space array: the family name, then its parameters, each
    /// copied as what that family takes there.
    fn color_space<'s>(
        &mut self,
        items: &'s [Object],
        scope: &[&'s Dictionary],
        depth: usize,
    ) -> Result<Option<Object>>
    where
        'a: 's,
    {
        let source = self.source;
        let Some((family, parameters)) = items.split_first() else {
            self.residue += 1;
            return Ok(None);
        };
        let Some(Object::Name(family)) = source.resolve(family) else {
            self.residue += 1;
            return Ok(None);
        };
        // Each family's parameters, and how many of them it requires.
        let (kinds, required): (&[Kind], usize) = match family.as_slice() {
            b"DeviceGray" | b"DeviceRGB" | b"DeviceCMYK" => (&[], 0),
            b"ICCBased" => (&[Kind::IccProfile], 1),
            b"Indexed" => (&[Kind::ColorSpace, Kind::Value, Kind::Data], 3),
            b"Separation" => (&[Kind::Value, Kind::ColorSpace, Kind::Function], 3),
            b"DeviceN" => (
                &[
                    Kind::Value,
                    Kind::ColorSpace,
                    Kind::Function,
                    Kind::DeviceNAttributes,
                ],
                3,
            ),
            b"Pattern" => (&[Kind::ColorSpace], 0),
            b"CalGray" | b"CalRGB" | b"Lab" => (&[Kind::CalParams], 1),
            _ => {
                self.residue += 1;
                return Ok(None);
            }
        };
        if parameters.len() < required {
            self.residue += 1;
            return Ok(None);
        }
        let mut out = vec![Object::Name(family.clone())];
        for (&kind, parameter) in kinds.iter().zip(parameters) {
            match self.copy(kind, parameter, scope, depth + 1)? {
                Some(parameter) => out.push(parameter),
                None => return Ok(None),
            }
        }
        self.residue += parameters.len().saturating_sub(kinds.len());
        Ok(Some(Object::Array(out)))
    }
}

/// A content stream holding `content`, compressed.
pub(super) fn compressed(content: Vec<u8>) -> Result<Stream> {
    let mut stream = Stream::new(Dictionary::new(), content);
    stream
        .compress()
        .map_err(|e| Error::malformed(format!("could not compress content: {e}")))?;
    Ok(stream)
}

/// Merge the resource dictionary `from` into `into`, category by category.
pub(super) fn merge(into: &mut Dictionary, from: Dictionary) {
    for (category, entries) in from {
        let Object::Dictionary(entries) = entries else {
            continue;
        };
        match into.get_mut(&category) {
            Ok(Object::Dictionary(existing)) => {
                for (name, value) in entries {
                    if existing.get(&name).is_err() {
                        existing.set(name, value);
                    }
                }
            }
            _ => into.set(category, entries),
        }
    }
}
