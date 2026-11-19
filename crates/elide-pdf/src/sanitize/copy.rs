//! Copying an object as its [`Kind`]: only the keys the kind keeps, every
//! value resolved, every child copied as the kind its key holds.

use std::mem;

use elide_core::redaction::HiddenContent;
use elide_core::{Error, ErrorKind, Result};
use hayro_syntax::object::{Array, Dict, MaybeRef, Object, Stream};

use super::keys::{self, FONT, FORM, Kind, SHADING_PATTERN, STREAM, TILING, TO_UNICODE};
use super::{Builder, MAX_COPIED_BYTES, MAX_NESTING, ScopedCopy, Src, VALUE_COST, record};
use crate::content::image;
use crate::pdf::decode::filters;
use crate::pdf::object::{Dictionary, Stream as Written, Value};
use crate::pdf::reader::{DictExt, ObjectId, Scope, StreamExt, id};
use crate::pdf::writer::{UnicodeMap, compressed, object_value};

/// Where a value is copied: the scope names in its content resolve in, how
/// deep in the rebuild it nests, and whether content drawn there begins
/// with no font to show text in.
#[derive(Debug, Clone, Copy)]
pub(super) struct At<'c, 's> {
    pub(super) scope: &'c Scope<'s>,
    pub(super) depth: usize,
    /// Whether text the content shows before it selects a font has none:
    /// content that starts its own graphics state (a page, a pattern's
    /// cell, a glyph, a flattened appearance), or a form drawn where its
    /// caller's font is lost. A form otherwise shows such text in its
    /// caller's font.
    pub(super) no_font: bool,
}

impl<'c, 's> At<'c, 's> {
    /// The top of a copy, in `scope`, with no font selected.
    pub(super) fn top(scope: &'c Scope<'s>) -> Self {
        Self {
            scope,
            depth: 0,
            no_font: true,
        }
    }

    /// The same place, its content beginning with no font when `no_font`.
    pub(super) fn font(self, no_font: bool) -> Self {
        Self { no_font, ..self }
    }

    /// One level deeper, in the same scope.
    pub(super) fn deeper(self) -> Self {
        Self {
            depth: self.depth + 1,
            ..self
        }
    }

    /// One level deeper, in `scope`: the content of a form, pattern or
    /// glyph, which resolves its names there.
    pub(super) fn within<'i>(self, scope: &'i Scope<'s>) -> At<'i, 's> {
        At {
            scope,
            depth: self.depth + 1,
            no_font: self.no_font,
        }
    }
}

impl<'a> Builder<'a> {
    /// `value` copied as `kind` at `at`, its names resolved in its scope
    /// where it is content; `None` when nothing of it is kept.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::ResourceLimit`](elide_core::ErrorKind::ResourceLimit) if
    /// objects nest past [`MAX_NESTING`] or the rebuild copies more than
    /// [`MAX_COPIED_BYTES`].
    pub(super) fn copy(
        &mut self,
        kind: Kind,
        value: &Src<'a>,
        at: At<'_, 'a>,
    ) -> Result<Option<Value>> {
        if at.depth > MAX_NESTING {
            return Err(Error::resource_limit(format!(
                "objects nest deeper than {MAX_NESTING} levels"
            )));
        }
        let object = match value {
            MaybeRef::Ref(reference) => {
                self.charge(VALUE_COST)?;
                return self.copy_reference(kind, id(*reference), at);
            }
            MaybeRef::NotRef(object) => object,
        };
        let held = match object {
            Object::String(string) => string.as_bytes().len(),
            Object::Name(name) => name.len(),
            _ => 0,
        };
        self.charge(VALUE_COST + held)?;
        match object {
            // A font held in place is written as an object of its own, as
            // the normal form has every font and stream.
            Object::Dict(_) | Object::Stream(_) => {
                Ok(match self.copy_object(kind, None, object, at)? {
                    Some(copy @ Value::Stream(_)) => Some(self.add(copy)),
                    Some(copy) if kind == Kind::Font => Some(self.add(copy)),
                    copy => copy,
                })
            }
            Object::Array(items) => {
                let items: Vec<Src<'a>> = items.raw_iter().collect();
                if kind == Kind::ColorSpace {
                    return self.color_space(&items, at);
                }
                let mut out = Vec::with_capacity(items.len());
                for item in &items {
                    match self.copy(kind, item, at.deeper())? {
                        Some(item) => out.push(item),
                        None => return Ok(None),
                    }
                }
                Ok(Some(Value::Array(out)))
            }
            Object::Null(_)
            | Object::Boolean(_)
            | Object::Number(_)
            | Object::Name(_)
            | Object::String(_) => Ok(Some(object_value(object))),
        }
    }

    /// The object `id` refers to copied as `kind`: a dictionary or stream as
    /// its own object, copied once per kind (and per scope, for an object
    /// whose copy depends on the scope it is drawn in); anything else in
    /// place.
    fn copy_reference(
        &mut self,
        kind: Kind,
        id: ObjectId,
        at: At<'_, 'a>,
    ) -> Result<Option<Value>> {
        let Some(object) = self.source.object(id) else {
            self.residue += 1;
            return Ok(None);
        };
        if !matches!(object, Object::Dict(_) | Object::Stream(_)) {
            return self.copy(kind, &MaybeRef::NotRef(object), at.deeper());
        }
        // An object whose copy depends on the scope it is drawn in (content
        // that resolves its names there, or an object holding such content)
        // is copied once for each scope whose resources differ; any other
        // object is copied once.
        let drawn_in = at.scope.dictionaries();
        // A form's copy also depends on whether its caller's font is lost.
        let key = (id, kind, kind == Kind::Form && at.no_font);
        let scopes = &self.scopes;
        let fits = |(index, _): &&ScopedCopy| {
            index.is_none_or(|index| scopes[index].as_slice() == drawn_in)
        };
        if let Some(&(index, copy)) = self
            .copies
            .get(&key)
            .and_then(|copies| copies.iter().find(fits))
        {
            self.scoped |= index.is_some();
            return Ok(Some(Value::Ref(copy)));
        }
        // The copy's id is taken first, so an object that refers back to
        // itself (a form drawing a pattern drawing the form) refers to it.
        let borrows = self.borrows_scope(kind, &object);
        let index = borrows.then(|| self.scope_index(drawn_in));
        let copy = self.out.reserve();
        self.copies.entry(key).or_default().push((index, copy));
        let outer = mem::take(&mut self.scoped);
        let copied = self.copy_object(kind, Some(id), &object, at.deeper())?;
        let scoped = borrows || self.scoped;
        self.scoped = outer || scoped;
        if scoped && index.is_none() {
            let index = self.scope_index(drawn_in);
            if let Some(entry) = self
                .copies
                .get_mut(&key)
                .and_then(|copies| copies.iter_mut().find(|(_, id)| *id == copy))
            {
                entry.0 = Some(index);
            }
        }
        match copied {
            Some(value) => {
                self.out.set(copy, value);
                Ok(Some(Value::Ref(copy)))
            }
            None => {
                if let Some(copies) = self.copies.get_mut(&key) {
                    copies.retain(|(_, id)| *id != copy);
                }
                Ok(None)
            }
        }
    }

    /// The index in [`Builder::scopes`] of the scope `resources`, kept there
    /// the first time it is seen. It is looked up only when a scoped copy is
    /// made, so the scan is paid once per copy, not once per reference.
    fn scope_index(&mut self, resources: &[Dict<'a>]) -> usize {
        self.scopes
            .iter()
            .position(|scope| scope.as_slice() == resources)
            .unwrap_or_else(|| {
                self.scopes.push(resources.to_vec());
                self.scopes.len() - 1
            })
    }

    /// Whether `object`, copied as `kind`, is content that resolves its names
    /// in the scope it is drawn in: a form, tiling pattern, or Type3 font
    /// whose `/Resources`, if any, is not a dictionary it can use.
    fn borrows_scope(&self, kind: Kind, object: &Object<'a>) -> bool {
        let (dict, stream) = match object {
            Object::Dict(dict) => (dict.clone(), false),
            Object::Stream(stream) => (stream.dict().clone(), true),
            _ => return false,
        };
        let content = match kind {
            Kind::Form | Kind::Pattern => stream,
            Kind::Font => dict.is_subtype(b"Type3"),
            _ => false,
        };
        content && Scope::own(&dict).is_none()
    }

    /// The dictionary or stream `value` (the source object `id`, when it is
    /// one) copied as `kind`.
    fn copy_object(
        &mut self,
        kind: Kind,
        id: Option<ObjectId>,
        value: &Object<'a>,
        at: At<'_, 'a>,
    ) -> Result<Option<Value>> {
        match (kind, value) {
            (Kind::Font, Object::Dict(dict)) => self.font(dict, at).map(Some),
            (Kind::Form, Object::Stream(stream)) => self.form(stream, FORM, at),
            (Kind::Pattern, _) => self.pattern(value, at),
            (Kind::FileSpec, Object::Dict(dict)) => self.file_spec(dict),
            (Kind::ToUnicode, Object::Stream(stream)) if !external(stream.dict()) => {
                self.unicode_map(stream, at)
            }
            (Kind::Image | Kind::Mask, Object::Stream(stream))
                if !image::is_surfaceable(stream) =>
            {
                self.unreadable(id);
                Ok(None)
            }
            (Kind::Image | Kind::Mask, Object::Stream(stream)) if !external(stream.dict()) => {
                self.image(kind, id, stream, at)
            }
            (Kind::Map(inner), Object::Dict(dict)) => {
                let mut out = Dictionary::new();
                for (key, value) in dict.entries() {
                    if let Some(value) = self.copy(*inner, &value, at.deeper())? {
                        out.set(&*key, value);
                    }
                }
                Ok(Some(Value::Dict(out)))
            }
            // A graphics state's font is carried as the `Tf` it selects, where
            // the state is set.
            (Kind::ExtGState, Object::Dict(dict)) => {
                let fields = keys::fields(kind).unwrap_or_default();
                Ok(Some(Value::Dict(self.keys(
                    dict,
                    fields,
                    &[b"Font"],
                    at,
                )?)))
            }
            _ => match keys::fields(kind) {
                Some(fields) => self.table(value, fields, at),
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
    fn own_scope(&mut self, dict: &Dict<'a>, scoped: bool) {
        if Scope::own(dict).is_some() {
            self.scoped = scoped;
        }
    }

    /// An image XObject or stencil mask `stream` (the source object `id`
    /// when it is one): one stored as raw samples written as exactly the
    /// samples it takes, so no bytes past them are carried; one stored as a
    /// JPEG or JPEG 2000 file copied as it is. Either is written as an image
    /// XObject, which the image pipeline surfaces, whatever its source said.
    fn image(
        &mut self,
        kind: Kind,
        id: Option<ObjectId>,
        stream: &Stream<'a>,
        at: At<'_, 'a>,
    ) -> Result<Option<Value>> {
        let fields = keys::fields(kind).unwrap_or_default();
        let samples = match image::raw_samples(stream) {
            Ok(Some(samples)) => {
                self.charge(samples.len())?;
                samples
            }
            Ok(None) => return self.stored(stream, fields, at),
            Err(_) => {
                self.unreadable(id);
                return Ok(None);
            }
        };
        let mut dict = self.stream_keys(
            stream.dict(),
            fields,
            &[b"Filter", b"DecodeParms", b"DL"],
            at,
        )?;
        image_xobject(&mut dict);
        Ok(Some(compressed(dict, &samples)?.into()))
    }

    /// An image `stream` stored as a JPEG or JPEG 2000 file, keeping
    /// `fields`, and its data and parameters as stored: the file a codec
    /// reads. Its one filter is written as a name.
    fn stored(
        &mut self,
        stream: &Stream<'a>,
        fields: &[(&[u8], Kind)],
        at: At<'_, 'a>,
    ) -> Result<Option<Value>> {
        let data = stream.raw_data().into_owned();
        self.charge(data.len())?;
        let mut dict = self.stream_keys(stream.dict(), fields, &[b"Filter", b"DecodeParms"], at)?;
        let Ok(filters) = filters(stream) else {
            return Ok(None);
        };
        let [filter] = filters.as_slice() else {
            return Ok(None);
        };
        let filter = filter.name.clone();
        dict.set("Filter", Value::Name(filter.clone()));
        // The one filter's parameters (a dictionary, or the one in a
        // one-item array): a JPEG's colour transform, 0 or 1, is the only
        // one either file filter takes (7.4.8), so it alone is written.
        let parameters = match stream.dict().get::<Object<'a>>(b"DecodeParms") {
            Some(Object::Array(parameters)) => {
                let parameters: Vec<Object<'a>> = parameters.iter::<Object<'a>>().collect();
                self.residue += parameters.len().saturating_sub(1);
                parameters.into_iter().next()
            }
            parameters => parameters,
        }
        .filter(|p| !matches!(p, Object::Null(_)));
        let transform = parameters
            .clone()
            .and_then(Object::into_dict)
            .and_then(|p| p.get::<i64>(b"ColorTransform"))
            .filter(|t| filter == b"DCTDecode" && matches!(t, 0 | 1));
        // Every other parameter, and a transform not written, is dropped.
        match parameters.map(Object::into_dict) {
            None => {}
            Some(Some(p)) => self.residue += p.len() - usize::from(transform.is_some()),
            Some(None) => self.residue += 1,
        }
        if let Some(transform) = transform {
            dict.set(
                "DecodeParms",
                Dictionary::from_iter([("ColorTransform", Value::Int(transform))]),
            );
        }
        image_xobject(&mut dict);
        Ok(Some(Written::new(dict, data).into()))
    }

    /// `value` added to the rebuild as an object of its own, referred to.
    fn add(&mut self, value: Value) -> Value {
        Value::Ref(self.out.add(value))
    }

    /// Take `bytes` from the copy budget.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::ResourceLimit`](elide_core::ErrorKind::ResourceLimit) if
    /// the rebuild would copy more than [`MAX_COPIED_BYTES`].
    pub(super) fn charge(&mut self, bytes: usize) -> Result<()> {
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
    fn table(
        &mut self,
        value: &Object<'a>,
        fields: &[(&[u8], Kind)],
        at: At<'_, 'a>,
    ) -> Result<Option<Value>> {
        Ok(Some(match value {
            Object::Dict(dict) => Value::Dict(self.keys(dict, fields, &[], at)?),
            Object::Stream(stream) if external(stream.dict()) => {
                self.record(HiddenContent::Unreadable);
                return Ok(None);
            }
            // Written as its data, compressed anew, so no encoding but Flate
            // reaches the output; its data decodes within what the copy
            // budget has left.
            Object::Stream(stream) => {
                let data = match stream.plain_within(self.copy_budget) {
                    Ok(data) => data,
                    Err(e) if e.kind() == ErrorKind::ResourceLimit => return Err(e),
                    Err(_) => {
                        self.record(HiddenContent::Unreadable);
                        return Ok(None);
                    }
                };
                self.charge(data.len())?;
                let dict = self.stream_keys(
                    stream.dict(),
                    fields,
                    &[b"Filter", b"DecodeParms", b"DL"],
                    at,
                )?;
                compressed(dict, &data)?.into()
            }
            _ => return Ok(None),
        }))
    }

    /// The entries of `dict` that `fields` keeps, each copied as its kind;
    /// every other entry but those in `handled` is recorded as dropped.
    pub(super) fn keys(
        &mut self,
        dict: &Dict<'a>,
        fields: &[(&[u8], Kind)],
        handled: &[&[u8]],
        at: At<'_, 'a>,
    ) -> Result<Dictionary> {
        let mut out = Dictionary::new();
        for (key, value) in dict.entries() {
            if handled.contains(&&*key) {
                continue;
            }
            match fields.iter().find(|(field, _)| *field == &*key) {
                Some(&(_, kind)) => {
                    if let Some(value) = self.copy(kind, &value, at.deeper())? {
                        out.set(&*key, value);
                    }
                }
                None => self.drop_key(&key, record::entry),
            }
        }
        Ok(out)
    }

    /// As [`keys`](Self::keys), for a stream's dictionary: the stream encoding
    /// entries are kept too, and its length is rewritten on save.
    fn stream_keys(
        &mut self,
        dict: &Dict<'a>,
        fields: &[(&[u8], Kind)],
        handled: &[&[u8]],
        at: At<'_, 'a>,
    ) -> Result<Dictionary> {
        let fields: Vec<(&[u8], Kind)> = fields.iter().chain(STREAM).copied().collect();
        let handled: Vec<&[u8]> = handled.iter().copied().chain([&b"Length"[..]]).collect();
        self.keys(dict, &fields, &handled, at)
    }

    /// A font, a Type3 font's glyph procedures rebuilt as content with names
    /// resolved in its own resources, else in `at`'s scope.
    fn font(&mut self, dict: &Dict<'a>, at: At<'_, 'a>) -> Result<Value> {
        let subtype = dict.name(b"Subtype");
        let subtype = subtype.as_deref();
        let handled: &[&[u8]] = match subtype {
            Some(b"Type3") => &[b"CharProcs", b"Resources"],
            Some(b"Type0") => &[b"Encoding"],
            _ => &[],
        };
        let mut out = self.keys(dict, FONT, handled, at)?;
        if subtype == Some(b"Type0") {
            if let Some(encoding) = dict.raw(b"Encoding")
                && let Some(encoding) = self.copy(Kind::CMap, &encoding, at.deeper())?
            {
                out.set("Encoding", encoding);
            }
            return Ok(Value::Dict(out));
        }
        if subtype != Some(b"Type3") {
            return Ok(Value::Dict(out));
        }
        let inner = at.scope.inner(dict);
        let scoped = self.scoped;
        // The glyph procedures share one resource dictionary, so the names
        // they are rebuilt with are given across all of them.
        let mut resources = Dictionary::new();
        let mut procs = Dictionary::new();
        if let Some(glyphs) = dict.get::<Dict<'a>>(b"CharProcs") {
            for name in glyphs.keys() {
                let Some(glyph) = glyphs.get::<Stream<'a>>(&*name) else {
                    self.residue += 1;
                    continue;
                };
                if external(glyph.dict()) {
                    self.record(HiddenContent::Unreadable);
                    continue;
                }
                let content = self.content(
                    &glyph.plain()?,
                    at.within(&inner).font(true),
                    &mut resources,
                )?;
                self.charge(content.len())?;
                let glyph = self.out.add(compressed(Dictionary::new(), &content)?);
                procs.set(&*name, glyph);
            }
        }
        self.own_scope(dict, scoped);
        out.set("CharProcs", procs);
        out.set("Resources", resources);
        Ok(Value::Dict(out))
    }

    /// A form XObject or tiling pattern `stream` keeping `fields`, its content
    /// rebuilt with names resolved in its own resources, else in `at`'s scope;
    /// `None`, recorded as unreadable, when its data is in another file or
    /// under a `/Crypt` filter.
    fn form(
        &mut self,
        stream: &Stream<'a>,
        fields: &[(&[u8], Kind)],
        at: At<'_, 'a>,
    ) -> Result<Option<Value>> {
        // Content kept in another file is not what the stream's own bytes
        // say, so it is not rebuilt from them.
        if external(stream.dict()) {
            self.record(HiddenContent::Unreadable);
            return Ok(None);
        }
        let inner = at.scope.inner(stream.dict());
        let scoped = self.scoped;
        let mut resources = Dictionary::new();
        let content = self.content(&stream.plain()?, at.within(&inner), &mut resources)?;
        // Content that borrows its scope is rebuilt once per scope, so each
        // rebuild is charged.
        self.charge(content.len())?;
        self.own_scope(stream.dict(), scoped);
        let mut dict = self.stream_keys(
            stream.dict(),
            fields,
            &[b"Resources", b"Filter", b"DecodeParms", b"DL"],
            at,
        )?;
        dict.set("Resources", resources);
        Ok(Some(compressed(dict, &content)?.into()))
    }

    /// A `/ToUnicode` CMap `stream` written anew from what detection reads
    /// of it, its destinations written as the text detection reads; dropped
    /// as unreadable when it can't be read, so a viewer reads no text
    /// detection did not.
    ///
    /// # Errors
    ///
    /// As [`UnicodeMap::read`] and [`UnicodeMap::readable`], and
    /// [`ErrorKind::ResourceLimit`](elide_core::ErrorKind::ResourceLimit) if
    /// the CMaps written across the document take more than
    /// [`MAX_CONTENT_BYTES`](crate::pdf::reader::MAX_CONTENT_BYTES).
    fn unicode_map(&mut self, stream: &Stream<'a>, at: At<'_, 'a>) -> Result<Option<Value>> {
        let Some(map) = UnicodeMap::read(stream)? else {
            self.record(HiddenContent::Unreadable);
            return Ok(None);
        };
        let content = map.readable()?.write();
        self.cmap_budget = self
            .cmap_budget
            .checked_sub(content.len())
            .ok_or_else(|| Error::resource_limit("the /ToUnicode CMaps pass their size limit"))?;
        let dict = self.stream_keys(
            stream.dict(),
            TO_UNICODE,
            &[b"Filter", b"DecodeParms", b"DL"],
            at,
        )?;
        Ok(Some(compressed(dict, &content)?.into()))
    }

    /// A pattern: a tiling one rebuilt as content, a shading one copied.
    fn pattern(&mut self, value: &Object<'a>, at: At<'_, 'a>) -> Result<Option<Value>> {
        // A pattern's kind is its shape: a stream is a tiling pattern, a
        // dictionary a shading pattern. `/PatternType` is written to match,
        // so a viewer and the text walk read the copy as the same kind.
        let (mut copy, kind) = match value {
            Object::Stream(stream) => match self.form(stream, TILING, at.font(true))? {
                Some(copy) => (copy, 1),
                None => return Ok(None),
            },
            Object::Dict(_) => match self.table(value, SHADING_PATTERN, at)? {
                Some(copy) => (copy, 2),
                None => return Ok(None),
            },
            _ => {
                self.residue += 1;
                return Ok(None);
            }
        };
        if let Some(dict) = copy.as_dict_mut() {
            dict.set("PatternType", kind);
        }
        Ok(Some(copy))
    }

    /// A colour space array: the family name, then its parameters, each
    /// copied as what that family takes there.
    fn color_space(&mut self, items: &[Src<'a>], at: At<'_, 'a>) -> Result<Option<Value>> {
        let Some((family, parameters)) = items.split_first() else {
            self.residue += 1;
            return Ok(None);
        };
        let Some(family) = self.source.resolve(family).and_then(Object::into_name) else {
            self.residue += 1;
            return Ok(None);
        };
        // Each family's parameters, and how many of them it requires.
        let (kinds, required): (&[Kind], usize) = match &*family {
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
        let mut out = vec![Value::Name(family.to_vec())];
        for (&kind, parameter) in kinds.iter().zip(parameters) {
            match self.copy(kind, parameter, at.deeper())? {
                Some(parameter) => out.push(parameter),
                None => return Ok(None),
            }
        }
        self.residue += parameters.len().saturating_sub(kinds.len());
        Ok(Some(Value::Array(out)))
    }
}

/// Whether the stream `dict` describes holds data the rebuild cannot carry:
/// data in another file (`/F`), or under a `/Crypt` filter.
pub(super) fn external(dict: &Dict<'_>) -> bool {
    let crypt = dict.name(b"Filter").is_some_and(|name| &*name == b"Crypt")
        || dict.get::<Array<'_>>(b"Filter").is_some_and(|names| {
            names
                .iter::<Object<'_>>()
                .any(|name| name.into_name().is_some_and(|name| &*name == b"Crypt"))
        });
    crypt || dict.contains_key(b"F")
}

/// Mark `dict` an image XObject.
fn image_xobject(dict: &mut Dictionary) {
    dict.set("Type", "XObject");
    dict.set("Subtype", "Image");
}
