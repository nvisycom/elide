//! Forms and tiling patterns drawn from a content stream, walked in place
//! under the state they inherit, within the walk's depth and total limits.

use elide_core::{Error, Result};
use lopdf::content::Content;
use lopdf::{Document, Object, ObjectId, Stream};

use super::geometry::{Matrix, TextState};
use super::stream::{read_matrix, walk_stream};
use super::{Inherited, MAX_XOBJECT_DEPTH, MAX_XOBJECT_WALKS, Walk};
use crate::content::text::StreamTarget;
use crate::pdf::reader::{DictionaryExt, ObjectExt, Scope, StreamExt};

/// Resolve and recurse into the Form XObject named `name` in `scope`.
///
/// Skips a non-form (e.g. image) XObject and a name absent from the scope,
/// neither of which draws text. A form's own `/Resources` become the scope for
/// its content; when it has none the parent scope stands in, as the spec
/// allows.
pub(super) fn walk_xobject<'a>(
    doc: &'a Document,
    name: &[u8],
    scope: &Scope<'a>,
    depth: u8,
    inherited: Inherited<'a>,
    ctx: &mut Walk<'_>,
) -> Result<()> {
    let Some(id) = scope
        .lookup(doc, b"XObject", name)
        .and_then(|xobject| xobject.as_reference().ok())
    else {
        return Ok(());
    };
    let Ok(Object::Stream(stream)) = doc.get_object(id) else {
        return Ok(());
    };
    // Only Form XObjects carry content to walk; images are not text.
    if !stream.dict.is_subtype(doc, b"Form") {
        return Ok(());
    }
    walk_nested(doc, id, stream, scope, depth, inherited, ctx)
}

/// Resolve and recurse into the tiling pattern named `name` in `scope`, the
/// pattern a `scn`/`SCN` selects to paint with.
///
/// A pattern's cell is drawn in its own graphics state, in the page's default
/// space transformed by the pattern's `/Matrix` (8.7.3.1), so it inherits no
/// font or transform. Its text is located once, however many times the cell is
/// tiled or the pattern selected, and deleting it from the one stream removes
/// it from every tile. A shading pattern draws no text.
pub(super) fn walk_pattern<'a>(
    doc: &'a Document,
    name: &[u8],
    scope: &Scope<'a>,
    depth: u8,
    ctx: &mut Walk<'_>,
) -> Result<()> {
    let Some(id) = scope
        .lookup(doc, b"Pattern", name)
        .and_then(|pattern| pattern.as_reference().ok())
    else {
        return Ok(());
    };
    let Ok(Object::Stream(stream)) = doc.get_object(id) else {
        return Ok(());
    };
    if stream
        .dict
        .get(b"PatternType")
        .ok()
        .and_then(ObjectExt::as_number)
        != Some(1.0)
        || !ctx.patterns.insert(id)
    {
        return Ok(());
    }
    let inherited = Inherited {
        font: None,
        state: TextState::default(),
    };
    walk_nested(doc, id, stream, scope, depth, inherited, ctx)
}

/// Walk the content of the form or tiling pattern `stream` (object `id`),
/// drawn from `scope` under the `inherited` state, its `/Matrix` composed onto
/// that state's transform.
///
/// Skips a re-entrant reference (cycle guard); content nested past
/// [`MAX_XOBJECT_DEPTH`], or more walks than [`MAX_XOBJECT_WALKS`], is an
/// error.
pub(super) fn walk_nested<'a>(
    doc: &'a Document,
    id: ObjectId,
    stream: &'a Stream,
    scope: &Scope<'a>,
    depth: u8,
    mut inherited: Inherited<'a>,
    ctx: &mut Walk<'_>,
) -> Result<()> {
    if ctx.active.contains(&id) {
        return Ok(());
    }
    if depth == 0 {
        return Err(Error::resource_limit(format!(
            "page {} nests forms and patterns deeper than {MAX_XOBJECT_DEPTH} levels; \
             its text cannot be walked",
            ctx.page
        )));
    }
    let content = Content::decode(&stream.decoded()?)
        .map_err(|e| Error::malformed(format!("decode XObject content: {e}")))?;

    // Its own resources, or the parent scope when it declares none.
    let inner_scope = scope.inner(doc, &stream.dict);

    // Charge the total-walk budget. Exhausting it means a page fans out to
    // more walks than any legitimate document, so fail closed rather than let
    // the walk run away.
    let Some(remaining) = ctx.walks_remaining.checked_sub(1) else {
        return Err(Error::resource_limit(format!(
            "page {} draws more than {MAX_XOBJECT_WALKS} forms and patterns; \
             its text cannot be walked within the resource budget",
            ctx.page
        )));
    };
    ctx.walks_remaining = remaining;

    // The `/Matrix` maps its content into the space it is drawn in (8.10.1,
    // 8.7.3.1); absent or malformed, it is the identity.
    let matrix = stream
        .dict
        .get(b"Matrix")
        .ok()
        .and_then(|o| o.as_array().ok())
        .and_then(|a| read_matrix(a, 0))
        .unwrap_or(Matrix::IDENTITY);
    inherited.state.ctm = matrix.then(inherited.state.ctm);

    ctx.active.insert(id);
    let result = walk_stream(
        doc,
        &content,
        StreamTarget::XObject(id),
        &inner_scope,
        depth - 1,
        Some(inherited),
        ctx,
    );
    ctx.active.remove(&id);
    result
}
