//! Rewriting a PDF's content streams: every page's (each set of shared
//! streams once), and every form XObject's, tiling pattern's and Type3 glyph
//! procedure's, planned against the document as it stands and written as
//! [`Changes`].
//!
//! Image removal cuts the drawing of an image this way. The document is a
//! rebuild, in the normal form: every font, form, pattern and glyph
//! procedure is an object of its own, and each page, form, pattern and Type3
//! font has its resources, every category of them, held in place.

use std::collections::{BTreeSet, HashSet};

use elide_core::Result;
use hayro_syntax::object::{Dict, Object, Stream};

use super::{Changes, compressed, dict_value, stream_value};
use crate::pdf::Pdf;
use crate::pdf::object::Value;
use crate::pdf::reader::{
    DictExt, ObjectId, Scope, StreamExt, page_content, page_stream_ids, page_streams,
};

/// Rewrite in `changes` every content stream of `pdf` that `rewrite`
/// changes: each page's (each set of shared streams once), and each form
/// XObject's, tiling pattern's and Type3 glyph procedure's. `rewrite` reads
/// a stream's bytes and the scope its names resolve in (a stream's own
/// resources, a glyph procedure's its font's), and returns its new bytes
/// and what it reports cutting; what each changed stream reports is
/// returned.
///
/// # Errors
///
/// As `rewrite`, and
/// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if
/// a stream cannot be read or written back.
pub(crate) fn rewrite_contents<'p, C>(
    pdf: &'p Pdf,
    changes: &mut Changes,
    mut rewrite: impl FnMut(&[u8], &Scope<'p>) -> Result<Option<(Vec<u8>, C)>>,
) -> Result<Vec<C>> {
    let mut cuts = Vec::new();
    // Pages can share content streams: rewrite each set once.
    let mut seen: BTreeSet<Vec<ObjectId>> = BTreeSet::new();
    for page in pdf.pages() {
        let streams = page_stream_ids(&page);
        if !seen.insert(streams.clone()) {
            continue;
        }
        let bytes = page_content(&page)?;
        let Some((content, cut)) = rewrite(&bytes, &Scope::page(&page))? else {
            continue;
        };
        write_page_content(&page, &content, changes)?;
        cuts.push(cut);
    }
    // Each content stream with the resources it resolves its names in.
    let mut rewritten = HashSet::new();
    for object in pdf.file().objects() {
        let (stream, scope): (Stream<'p>, Scope<'p>) = match object {
            Object::Stream(stream) if is_content(stream.dict()) => {
                let scope = Scope::new(Scope::own(stream.dict()));
                (stream, scope)
            }
            Object::Dict(font) if font.is_subtype(b"Type3") => {
                let scope = Scope::new(Scope::own(&font));
                for glyph in glyphs(&font) {
                    if rewritten.insert(glyph.obj_id()) {
                        cuts.extend(rewrite_stream(&glyph, &scope, changes, &mut rewrite)?);
                    }
                }
                continue;
            }
            _ => continue,
        };
        if rewritten.insert(stream.obj_id()) {
            cuts.extend(rewrite_stream(&stream, &scope, changes, &mut rewrite)?);
        }
    }
    Ok(cuts)
}

/// Write `content` as the content of `page` into `changes`: into its first
/// content stream, the page naming that one alone when its content was
/// split over several.
///
/// # Errors
///
/// As [`compressed`].
pub(crate) fn write_page_content(
    page: &Dict<'_>,
    content: &[u8],
    changes: &mut Changes,
) -> Result<()> {
    let streams = page_streams(page);
    let Some((first, rest)) = streams.split_first() else {
        return Ok(());
    };
    changes.replace(
        first.obj_id(),
        compressed(stream_value(first).dict, content)?,
    );
    if !rest.is_empty()
        && let Some(id) = page.obj_id()
    {
        let mut dict = dict_value(page);
        dict.set("Contents", Value::Ref(super::copied(first.obj_id())));
        changes.replace(id, dict);
    }
    Ok(())
}

/// Rewrite the content `stream` in `scope` into `changes`, keeping its
/// dictionary, as `rewrite` changes it.
fn rewrite_stream<'p, C>(
    stream: &Stream<'p>,
    scope: &Scope<'p>,
    changes: &mut Changes,
    rewrite: &mut impl FnMut(&[u8], &Scope<'p>) -> Result<Option<(Vec<u8>, C)>>,
) -> Result<Option<C>> {
    let bytes = stream.plain()?;
    let Some((content, cut)) = rewrite(&bytes, scope)? else {
        return Ok(None);
    };
    let id = stream.obj_id();
    // A dictionary another change already rewrote is kept as it was
    // rewritten.
    let dict = match changes.replacement(id) {
        Some(Value::Stream(written)) => written.dict.clone(),
        _ => stream_value(stream).dict,
    };
    changes.replace(id, compressed(dict, &content)?);
    Ok(Some(cut))
}

/// Whether the stream `dict` is content: a form XObject's or a tiling
/// pattern's.
fn is_content(dict: &Dict<'_>) -> bool {
    dict.is_subtype(b"Form") || dict.number(b"PatternType") == Some(1.0)
}

/// The glyph procedures of the Type3 `font`.
fn glyphs<'p>(font: &Dict<'p>) -> Vec<Stream<'p>> {
    let Some(procs) = font.get::<Dict<'p>>(b"CharProcs") else {
        return Vec::new();
    };
    procs
        .keys()
        .filter_map(|name| procs.get::<Stream<'p>>(&*name))
        .collect()
}

/// Drop in `changes` every entry naming one of the `hidden` XObjects from
/// the `/XObject` resources of every page, form, pattern and Type3 font of
/// `pdf`.
pub(crate) fn drop_xobjects(pdf: &Pdf, hidden: &HashSet<ObjectId>, changes: &mut Changes) {
    if hidden.is_empty() {
        return;
    }
    let held = |dict: &Dict<'_>| {
        dict.get::<Dict<'_>>(b"Resources")
            .and_then(|resources| resources.get::<Dict<'_>>(b"XObject"))
            .is_some_and(|entries| {
                entries.keys().any(|name| {
                    entries
                        .reference(&name)
                        .is_some_and(|id| hidden.contains(&id))
                })
            })
    };
    for object in pdf.file().objects() {
        let (id, dict, stream) = match &object {
            Object::Dict(dict) => (dict.obj_id(), dict.clone(), None),
            Object::Stream(stream) => (Some(stream.obj_id()), stream.dict().clone(), Some(stream)),
            _ => continue,
        };
        let Some(id) = id.filter(|_| held(&dict)) else {
            continue;
        };
        let written = changes.entry(id, || match stream {
            Some(stream) => Value::from(stream_value(stream)),
            None => Value::Dict(dict_value(&dict)),
        });
        let Some(entries) = written
            .as_dict_mut()
            .and_then(|dict| dict.get_mut(b"Resources"))
            .and_then(Value::as_dict_mut)
            .and_then(|resources| resources.get_mut(b"XObject"))
            .and_then(Value::as_dict_mut)
        else {
            continue;
        };
        entries.retain(|_, value| {
            value
                .as_ref_id()
                .is_none_or(|id| !hidden.iter().any(|h| super::copied(*h) == id))
        });
    }
}
