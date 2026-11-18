//! Copying an object as its [`Kind`]: only the keys the kind keeps, every
//! value resolved, every child copied as the kind its key holds.

use std::mem;

use elide_core::redaction::HiddenContent;
use elide_core::{Error, ErrorKind, Result};
use lopdf::{Dictionary, Object, ObjectId, Stream};

use super::keys::{self, FONT, FORM, Kind, SHADING_PATTERN, STREAM, TILING, TO_UNICODE};
use super::{Builder, MAX_COPIED_BYTES, MAX_NESTING, ScopedCopy, VALUE_COST, record};
use crate::content::image;
use crate::pdf::reader::{DictionaryExt, DocumentExt, ObjectExt, Scope, StreamExt};
use crate::pdf::writer::{UnicodeMap, compressed};

impl<'a> Builder<'a> {
    /// `value` copied as `kind`, its names resolved in `scope` where it is
    /// content; `None` when nothing of it is kept.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::ResourceLimit`](elide_core::ErrorKind::ResourceLimit) if
    /// objects nest past [`MAX_NESTING`] or the rebuild copies more than
    /// [`MAX_COPIED_BYTES`].
    pub(super) fn copy<'s>(
        &mut self,
        kind: Kind,
        value: &'s Object,
        scope: &Scope<'s>,
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
        let held = match value {
            Object::String(bytes, _) | Object::Name(bytes) => bytes.len(),
            _ => 0,
        };
        self.charge(VALUE_COST + held)?;
        match value {
            Object::Reference(id) => self.copy_reference(kind, *id, scope, depth),
            // A font held in place is written as an object of its own, as
            // the normal form has every font and stream.
            Object::Dictionary(_) | Object::Stream(_) => {
                Ok(match self.copy_object(kind, None, value, scope, depth)? {
                    Some(copy @ Object::Stream(_)) => Some(self.add(copy)),
                    Some(copy) if kind == Kind::Font => Some(self.add(copy)),
                    copy => copy,
                })
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
    /// its own object, copied once per kind (and per scope, for an object
    /// whose copy depends on the scope it is drawn in); anything else in
    /// place.
    fn copy_reference<'s>(
        &mut self,
        kind: Kind,
        id: ObjectId,
        scope: &Scope<'s>,
        depth: usize,
    ) -> Result<Option<Object>>
    where
        'a: 's,
    {
        // The object `id` holds, or the one a chain of references from it
        // ends at.
        let source = self.source;
        let Some(Ok((last, object))) = source.objects.get(&id).map(|o| source.dereference(o))
        else {
            self.residue += 1;
            return Ok(None);
        };
        let target = last.unwrap_or(id);
        if !matches!(object, Object::Dictionary(_) | Object::Stream(_)) {
            return self.copy(kind, object, scope, depth + 1);
        }
        // An object whose copy depends on the scope it is drawn in (content
        // that resolves its names there, or an object holding such content)
        // is copied once for each scope whose resources differ; any other
        // object is copied once.
        let drawn_in = scope.dictionaries();
        let scopes = &self.scopes;
        let fits = |(index, _): &&ScopedCopy| {
            index.is_none_or(|index| scopes[index].iter().eq(drawn_in.iter().copied()))
        };
        if let Some(&(index, copy)) = self
            .copies
            .get(&(target, kind))
            .and_then(|copies| copies.iter().find(fits))
        {
            self.scoped |= index.is_some();
            return Ok(Some(Object::Reference(copy)));
        }
        // The copy's id is taken first, so an object that refers back to
        // itself (a form drawing a pattern drawing the form) refers to it.
        let borrows = self.borrows_scope(kind, object);
        let index = borrows.then(|| self.scope_index(drawn_in));
        let copy = self.out.new_object_id();
        self.copies
            .entry((target, kind))
            .or_default()
            .push((index, copy));
        let outer = mem::take(&mut self.scoped);
        let copied = self.copy_object(kind, Some(target), object, scope, depth + 1)?;
        let scoped = borrows || self.scoped;
        self.scoped = outer || scoped;
        if scoped && index.is_none() {
            let index = self.scope_index(drawn_in);
            if let Some(entry) = self
                .copies
                .get_mut(&(target, kind))
                .and_then(|copies| copies.iter_mut().find(|(_, id)| *id == copy))
            {
                entry.0 = Some(index);
            }
        }
        match copied {
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

    /// The index in [`Builder::scopes`] of the scope `resources`, kept there
    /// the first time it is seen. It is looked up only when a scoped copy is
    /// made, so the scan is paid once per copy, not once per reference.
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
        let Some(dict) = object.dict() else {
            return false;
        };
        let content = match kind {
            Kind::Form | Kind::Pattern => matches!(object, Object::Stream(_)),
            Kind::Font => dict.is_subtype(self.source, b"Type3"),
            _ => false,
        };
        content && Scope::own(self.source, dict).is_none()
    }

    /// The dictionary or stream `value` (the source object `id`, when it is
    /// one) copied as `kind`.
    fn copy_object<'s>(
        &mut self,
        kind: Kind,
        id: Option<ObjectId>,
        value: &'s Object,
        scope: &Scope<'s>,
        depth: usize,
    ) -> Result<Option<Object>>
    where
        'a: 's,
    {
        match (kind, value) {
            (Kind::Font, Object::Dictionary(dict)) => self.font(dict, scope, depth).map(Some),
            (Kind::Form, Object::Stream(stream)) => self.form(stream, FORM, scope, depth),
            (Kind::Pattern, _) => self.pattern(value, scope, depth),
            (Kind::FileSpec, Object::Dictionary(dict)) => self.file_spec(dict),
            (Kind::ToUnicode, Object::Stream(stream)) if !self.external(&stream.dict) => {
                self.unicode_map(stream, scope, depth)
            }
            (Kind::Image | Kind::Mask, Object::Stream(stream))
                if !image::is_surfaceable(self.source, stream) =>
            {
                self.unreadable(id);
                Ok(None)
            }
            (Kind::Image | Kind::Mask, Object::Stream(stream)) if !self.external(&stream.dict) => {
                self.image(kind, id, stream, scope, depth)
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
            // A graphics state's font is carried as the `Tf` it selects, where
            // the state is set.
            (Kind::ExtGState, Object::Dictionary(dict)) => {
                let fields = keys::fields(kind).unwrap_or_default();
                Ok(Some(Object::Dictionary(self.keys(
                    dict,
                    fields,
                    &[b"Font"],
                    scope,
                    depth,
                )?)))
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

    /// After the content `dict` is rebuilt: when it has resources of its own,
    /// what it draws resolved its names there, not in the scope it is drawn
    /// in, so whether its copy depends on that scope is as it was before,
    /// `scoped`.
    fn own_scope(&mut self, dict: &Dictionary, scoped: bool) {
        if Scope::own(self.source, dict).is_some() {
            self.scoped = scoped;
        }
    }

    /// An image XObject or stencil mask `stream` (the source object `id`
    /// when it is one): one stored as raw samples written as exactly the
    /// samples it takes, so no bytes past them are carried; one stored as a
    /// JPEG or JPEG 2000 file copied as it is.
    fn image<'s>(
        &mut self,
        kind: Kind,
        id: Option<ObjectId>,
        stream: &'s Stream,
        scope: &Scope<'s>,
        depth: usize,
    ) -> Result<Option<Object>>
    where
        'a: 's,
    {
        let fields = keys::fields(kind).unwrap_or_default();
        let samples = match image::raw_samples(self.source, stream) {
            Ok(Some(samples)) => {
                self.charge(samples.len())?;
                samples
            }
            Ok(None) => return self.stored(stream, fields, scope, depth),
            Err(_) => {
                self.unreadable(id);
                return Ok(None);
            }
        };
        let dict = self.stream_keys(
            &stream.dict,
            fields,
            &[b"Filter", b"DecodeParms", b"DL"],
            scope,
            depth,
        )?;
        Ok(Some(Object::Stream(compressed(dict, samples)?)))
    }

    /// An image `stream` stored as a JPEG or JPEG 2000 file, keeping
    /// `fields`, its data and encoding as stored: the file a codec reads.
    fn stored<'s>(
        &mut self,
        stream: &'s Stream,
        fields: &[(&[u8], Kind)],
        scope: &Scope<'s>,
        depth: usize,
    ) -> Result<Option<Object>>
    where
        'a: 's,
    {
        self.charge(stream.content.len())?;
        let dict = self.stream_keys(&stream.dict, fields, &[], scope, depth)?;
        Ok(Some(Object::Stream(
            Stream::new(dict, stream.content.clone()).with_compression(false),
        )))
    }

    /// `object` added to the rebuild as an object of its own, referred to.
    fn add(&mut self, object: Object) -> Object {
        Object::Reference(self.out.add_object(object))
    }

    /// Take `bytes` from the copy budget.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::ResourceLimit`](elide_core::ErrorKind::ResourceLimit) if
    /// the rebuild would copy more than [`MAX_COPIED_BYTES`].
    fn charge(&mut self, bytes: usize) -> Result<()> {
        self.copy_budget = self.copy_budget.checked_sub(bytes).ok_or_else(|| {
            Error::resource_limit(format!(
                "the rebuild would copy more than {MAX_COPIED_BYTES} bytes"
            ))
        })?;
        Ok(())
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
        scope: &Scope<'s>,
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
            // Written as its data, compressed anew, so no encoding but Flate
            // reaches the output; its data decodes within what the copy
            // budget has left.
            Object::Stream(stream) => {
                let data = match stream.decoded_within(self.copy_budget) {
                    Ok(data) => data,
                    Err(e) if e.kind() == ErrorKind::ResourceLimit => return Err(e),
                    Err(_) => {
                        self.record(HiddenContent::Unreadable);
                        return Ok(None);
                    }
                };
                self.charge(data.len())?;
                let dict = self.stream_keys(
                    &stream.dict,
                    fields,
                    &[b"Filter", b"DecodeParms", b"DL"],
                    scope,
                    depth,
                )?;
                Object::Stream(compressed(dict, data)?)
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
        scope: &Scope<'s>,
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
        scope: &Scope<'s>,
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
    fn font<'s>(&mut self, dict: &'s Dictionary, scope: &Scope<'s>, depth: usize) -> Result<Object>
    where
        'a: 's,
    {
        let source = self.source;
        let subtype = dict.name(self.source, b"Subtype");
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
        let inner = scope.inner(source, dict);
        let scoped = self.scoped;
        // The glyph procedures share one resource dictionary, so the names
        // they are rebuilt with are given across all of them.
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
            if self.external(&glyph.dict) {
                self.record(HiddenContent::Unreadable);
                continue;
            }
            let content = self.content(&glyph.decoded()?, &inner, &mut resources, depth + 1)?;
            self.charge(content.len())?;
            procs.set(
                name.clone(),
                self.out.add_object(compressed(Dictionary::new(), content)?),
            );
        }
        self.own_scope(dict, scoped);
        out.set("CharProcs", procs);
        out.set("Resources", resources);
        Ok(Object::Dictionary(out))
    }

    /// A form XObject or tiling pattern `stream` keeping `fields`, its content
    /// rebuilt with names resolved in its own resources, else in `scope`;
    /// `None`, recorded as unreadable, when its data is in another file or
    /// under a `/Crypt` filter.
    fn form<'s>(
        &mut self,
        stream: &'s Stream,
        fields: &[(&[u8], Kind)],
        scope: &Scope<'s>,
        depth: usize,
    ) -> Result<Option<Object>>
    where
        'a: 's,
    {
        // Content kept in another file is not what the stream's own bytes
        // say, so it is not rebuilt from them.
        if self.external(&stream.dict) {
            self.record(HiddenContent::Unreadable);
            return Ok(None);
        }
        let inner = scope.inner(self.source, &stream.dict);
        let scoped = self.scoped;
        let mut resources = Dictionary::new();
        let content = self.content(&stream.decoded()?, &inner, &mut resources, depth + 1)?;
        // Content that borrows its scope is rebuilt once per scope, so each
        // rebuild is charged.
        self.charge(content.len())?;
        self.own_scope(&stream.dict, scoped);
        let mut dict = self.stream_keys(
            &stream.dict,
            fields,
            &[b"Resources", b"Filter", b"DecodeParms", b"DL"],
            scope,
            depth,
        )?;
        dict.set("Resources", resources);
        Ok(Some(Object::Stream(compressed(dict, content)?)))
    }

    /// A `/ToUnicode` CMap `stream` written anew from what lopdf, which
    /// detection decodes text with, reads of it, its destinations written as
    /// the text detection reads; dropped as unreadable when lopdf can't read
    /// it, so a viewer reads no text detection did not.
    ///
    /// # Errors
    ///
    /// As [`UnicodeMap::read`], and
    /// [`ErrorKind::ResourceLimit`](elide_core::ErrorKind::ResourceLimit) if
    /// the CMaps written across the document take more than
    /// [`MAX_CONTENT_BYTES`](crate::pdf::reader::MAX_CONTENT_BYTES).
    fn unicode_map<'s>(
        &mut self,
        stream: &'s Stream,
        scope: &Scope<'s>,
        depth: usize,
    ) -> Result<Option<Object>>
    where
        'a: 's,
    {
        let Some(map) = UnicodeMap::read(self.source, stream)? else {
            self.record(HiddenContent::Unreadable);
            return Ok(None);
        };
        let content = map.readable().write();
        self.cmap_budget = self
            .cmap_budget
            .checked_sub(content.len())
            .ok_or_else(|| Error::resource_limit("the /ToUnicode CMaps pass their size limit"))?;
        let dict = self.stream_keys(
            &stream.dict,
            TO_UNICODE,
            &[b"Filter", b"DecodeParms", b"DL"],
            scope,
            depth,
        )?;
        Ok(Some(Object::Stream(compressed(dict, content)?)))
    }

    /// A pattern: a tiling one rebuilt as content, a shading one copied.
    fn pattern<'s>(
        &mut self,
        value: &'s Object,
        scope: &Scope<'s>,
        depth: usize,
    ) -> Result<Option<Object>>
    where
        'a: 's,
    {
        // A pattern's kind is its shape: a stream is a tiling pattern, a
        // dictionary a shading pattern. `/PatternType` is written to match,
        // so a viewer and the text walk read the copy as the same kind.
        let (mut copy, kind) = match value {
            Object::Stream(stream) => match self.form(stream, TILING, scope, depth)? {
                Some(copy) => (copy, 1),
                None => return Ok(None),
            },
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
        scope: &Scope<'s>,
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
