//! Rewrite a PDF's content streams: every page's (each set of shared streams
//! once), and every form XObject's, tiling pattern's and Type3 glyph
//! procedure's, planned against the unchanged document and then written back.
//!
//! Image removal cuts the drawing of an image this way.

use std::collections::{BTreeSet, HashSet};

use elide_core::{Error, Result};
use lopdf::{Dictionary, Document, Object, ObjectId, Stream};

use crate::pdf::reader::{DictionaryExt, DocumentExt, ObjectExt, Scope, StreamExt};

/// Where a content stream is written back: a page's content, or the stream
/// object of a form XObject, tiling pattern or glyph procedure.
pub(super) enum Target {
    Page(ObjectId),
    Stream(ObjectId),
}

/// The content streams of `doc`'s pages (each set of shared streams once),
/// form XObjects, tiling patterns and Type3 glyph procedures that `rewrite`
/// changes, with their new content and what `rewrite` reports cutting.
/// `rewrite` reads a stream's bytes and the resource dictionaries its names
/// resolve against: a stream's own, a glyph procedure's its font's.
pub(super) fn plan<C>(
    doc: &Document,
    mut rewrite: impl FnMut(&Document, &[u8], &Scope<'_>) -> Result<Option<(Vec<u8>, C)>>,
) -> Result<Vec<(Target, Vec<u8>, C)>> {
    let mut edits = Vec::new();
    // Pages can share content streams: rewrite each set once.
    let mut seen: BTreeSet<Vec<ObjectId>> = BTreeSet::new();
    for page in doc.get_pages().into_values() {
        if !seen.insert(doc.get_page_contents(page)) {
            continue;
        }
        let bytes = doc.page_content(page)?;
        if let Some((content, cut)) = rewrite(doc, &bytes, &Scope::page(doc, page))? {
            edits.push((Target::Page(page), content, cut));
        }
    }
    // Each content stream with the resources it resolves its names in.
    let mut streams: Vec<(ObjectId, &Stream, Option<&Dictionary>)> = Vec::new();
    for (&id, object) in &doc.objects {
        match object {
            Object::Stream(stream)
                if stream.dict.is_subtype(doc, b"Form") || is_tiling(doc, &stream.dict) =>
            {
                streams.push((id, stream, Scope::own(doc, &stream.dict)));
            }
            Object::Dictionary(font) if font.is_subtype(doc, b"Type3") => {
                let procs = font
                    .get(b"CharProcs")
                    .ok()
                    .and_then(|procs| doc.resolve_dict(procs));
                for (_, glyph) in procs.into_iter().flat_map(Dictionary::iter) {
                    if let Ok(id) = glyph.as_reference()
                        && let Ok(Object::Stream(stream)) = doc.get_object(id)
                    {
                        streams.push((id, stream, Scope::own(doc, font)));
                    }
                }
            }
            _ => {}
        }
    }
    let mut rewritten = HashSet::new();
    for (id, stream, resources) in streams {
        if !rewritten.insert(id) {
            continue;
        }
        let bytes = stream.decoded()?;
        if let Some((content, cut)) = rewrite(doc, &bytes, &Scope::new(resources))? {
            edits.push((Target::Stream(id), content, cut));
        }
    }
    Ok(edits)
}

/// Write each planned content stream back, returning what each cut.
pub(super) fn apply<C>(doc: &mut Document, edits: Vec<(Target, Vec<u8>, C)>) -> Result<Vec<C>> {
    let mut cuts = Vec::with_capacity(edits.len());
    for (target, content, cut) in edits {
        match target {
            Target::Page(page) => doc
                .change_page_content(page, content)
                .map_err(|e| Error::malformed(format!("write page content: {e}")))?,
            Target::Stream(id) => doc.set_stream_content(id, content)?,
        }
        cuts.push(cut);
    }
    Ok(cuts)
}

/// Whether the stream `dict` is a tiling pattern's.
fn is_tiling(doc: &Document, dict: &Dictionary) -> bool {
    dict.get(b"PatternType")
        .ok()
        .and_then(|kind| doc.resolve_number(kind))
        == Some(1.0)
}

/// Drop every entry naming one of the `hidden` XObjects from the `/XObject`
/// sub-dictionary of every resource dictionary, inline or referenced.
pub(super) fn drop_xobjects(doc: &mut Document, hidden: &HashSet<ObjectId>) {
    if hidden.is_empty() {
        return;
    }
    let retain = |entries: &mut Dictionary| {
        let names: Vec<Vec<u8>> = entries
            .iter()
            .filter(|(_, value)| value.as_reference().is_ok_and(|id| hidden.contains(&id)))
            .map(|(name, _)| name.clone())
            .collect();
        for name in names {
            entries.remove(&name);
        }
    };
    // The `/XObject` entry of the resource dictionary `resources`: drop inline
    // entries in place, and collect a referenced sub-dictionary for later.
    let mut referenced: Vec<ObjectId> = Vec::new();
    let mut visit = |resources: &mut Dictionary| match resources.get_mut(b"XObject") {
        Ok(Object::Dictionary(entries)) => retain(entries),
        Ok(Object::Reference(id)) => referenced.push(*id),
        _ => {}
    };
    // A resource dictionary is an object of its own, or inline in its owner.
    for dict in doc.objects.values_mut().filter_map(ObjectExt::dict_mut) {
        if let Ok(Object::Dictionary(inline)) = dict.get_mut(b"Resources") {
            visit(inline);
        }
        visit(dict);
    }
    for id in referenced {
        if let Ok(Object::Dictionary(entries)) = doc.get_object_mut(id) {
            retain(entries);
        }
    }
}
