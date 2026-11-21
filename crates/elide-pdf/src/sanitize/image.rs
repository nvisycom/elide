//! Images as the rebuild copies them: raw samples written as exactly the
//! samples the image takes, a JPEG or JPEG 2000 file as it is stored.

use elide_core::Result;
use lopdf::{Dictionary, Object, ObjectId, Stream};

use super::Builder;
use super::copy::At;
use super::keys::{self, Kind};
use crate::content::image;
use crate::pdf::reader::DocumentExt;
use crate::pdf::writer::compressed;

impl<'a> Builder<'a> {
    /// An image XObject or stencil mask `stream` (the source object `id`
    /// when it is one): one stored as raw samples written as exactly the
    /// samples it takes, so no bytes past them are carried; one stored as a
    /// JPEG or JPEG 2000 file copied as it is. Either is written as an image
    /// XObject, which the image pipeline surfaces, whatever its source said.
    pub(super) fn image<'s>(
        &mut self,
        kind: Kind,
        id: Option<ObjectId>,
        stream: &'s Stream,
        at: At<'_, 's>,
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
            Ok(None) => return self.stored(id, stream, fields, at),
            Err(_) => {
                self.unreadable(id);
                return Ok(None);
            }
        };
        let mut dict = self.stream_keys(
            &stream.dict,
            fields,
            &[b"Filter", b"DecodeParms", b"DL"],
            at,
        )?;
        image_xobject(&mut dict);
        Ok(Some(Object::Stream(compressed(dict, samples)?)))
    }

    /// An image `stream` (the source object `id` when it is one) stored as a
    /// JPEG or JPEG 2000 file, keeping `fields`, and its data and parameters
    /// as stored: the file a codec reads. Its one filter is written as a
    /// name; one stored under any other is recorded as unreadable.
    fn stored<'s>(
        &mut self,
        id: Option<ObjectId>,
        stream: &'s Stream,
        fields: &[(&[u8], Kind)],
        at: At<'_, 's>,
    ) -> Result<Option<Object>>
    where
        'a: 's,
    {
        self.charge(stream.content.len())?;
        let mut dict = self.stream_keys(&stream.dict, fields, &[b"Filter", b"DecodeParms"], at)?;
        let filters = stream.filters();
        let Ok([filter]) = filters.as_deref() else {
            self.unreadable(id);
            return Ok(None);
        };
        dict.set("Filter", Object::Name(filter.to_vec()));
        // The one filter's parameters (a dictionary, or the one in a
        // one-item array): a JPEG's colour transform, 0 or 1, is the only
        // one either file filter takes (7.4.8), so it alone is written.
        let source = self.source;
        let parameters = match stream
            .dict
            .get(b"DecodeParms")
            .ok()
            .and_then(|p| source.resolve(p))
        {
            Some(Object::Array(parameters)) => {
                self.residue += parameters.len().saturating_sub(1);
                parameters.first().and_then(|p| source.resolve(p))
            }
            parameters => parameters,
        }
        .filter(|p| !matches!(p, Object::Null));
        let transform = parameters
            .and_then(|p| p.as_dict().ok())
            .and_then(|p| source.resolve(p.get(b"ColorTransform").ok()?))
            .and_then(|t| t.as_i64().ok())
            .filter(|t| *filter == b"DCTDecode" && matches!(t, 0 | 1));
        // Every other parameter, and a transform not written, is dropped.
        match parameters.map(Object::as_dict) {
            None => {}
            Some(Ok(p)) => self.residue += p.len() - usize::from(transform.is_some()),
            Some(Err(_)) => self.residue += 1,
        }
        if let Some(transform) = transform {
            dict.set(
                "DecodeParms",
                Dictionary::from_iter([("ColorTransform", Object::Integer(transform))]),
            );
        }
        image_xobject(&mut dict);
        Ok(Some(Object::Stream(
            Stream::new(dict, stream.content.clone()).with_compression(false),
        )))
    }
}

/// Mark `dict` an image XObject.
fn image_xobject(dict: &mut Dictionary) {
    dict.set("Type", Object::Name(b"XObject".to_vec()));
    dict.set("Subtype", Object::Name(b"Image".to_vec()));
}
