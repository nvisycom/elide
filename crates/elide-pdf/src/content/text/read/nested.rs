//! Forms and tiling patterns drawn from a content stream, walked in place
//! under the state they inherit, within the walk's depth and total limits.

use elide_core::{Error, Result};
use lopdf::content::Content;
use lopdf::{Document, Object, ObjectId, Stream};

use super::geometry::TextState;
use super::run::gap;
use super::stream::walk_stream;
use super::{Inherited, MAX_XOBJECT_DEPTH, MAX_XOBJECT_WALKS, Walk};
use crate::content::text::StreamTarget;
use crate::pdf::reader::{DictionaryExt, DocumentExt, Matrix, ObjectExt, Scope, StreamExt};

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
    inherited: Inherited<'_>,
    ctx: &mut Walk<'_, 'a>,
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

/// A tiling pattern selected to paint with, to be walked once the page's own
/// text is: the pattern, the scope it was selected in, and the nesting depth
/// left there.
pub(super) struct Selected<'d> {
    id: ObjectId,
    stream: &'d Stream,
    scope: Scope<'d>,
    depth: u8,
}

/// Select the tiling pattern named `name` in `scope`, the pattern a
/// `scn`/`SCN` selects to paint with, to be walked once the page's own text
/// is; one already selected on the page, or a shading pattern, which draws
/// no text, is passed over.
pub(super) fn select_pattern<'d>(
    doc: &'d Document,
    name: &[u8],
    scope: &Scope<'d>,
    depth: u8,
    ctx: &mut Walk<'_, 'd>,
) {
    let Some(id) = scope
        .lookup(doc, b"Pattern", name)
        .and_then(|pattern| pattern.as_reference().ok())
    else {
        return;
    };
    let Ok(Object::Stream(stream)) = doc.get_object(id) else {
        return;
    };
    if stream
        .dict
        .get(b"PatternType")
        .ok()
        .and_then(ObjectExt::as_number)
        == Some(1.0)
        && ctx.patterns.insert(id)
    {
        ctx.selected.push(Selected {
            id,
            stream,
            scope: scope.clone(),
            depth,
        });
    }
}

/// Walk the selected tiling `pattern`.
///
/// A pattern's cell is drawn in its own graphics state, in the page's default
/// space transformed by the pattern's `/Matrix` (8.7.3.1), so it inherits no
/// font, transform or pen. Its text is located once, however many times the
/// cell is tiled or the pattern selected, and deleting it from the one stream
/// removes it from every tile. Text it draws is set apart from the text
/// around it by a gap on each side; one that draws none leaves no gap.
pub(super) fn walk_pattern<'d>(
    doc: &'d Document,
    pattern: Selected<'d>,
    ctx: &mut Walk<'_, 'd>,
) -> Result<()> {
    let inherited = Inherited {
        font: None,
        state: TextState::default(),
    };
    let before = *ctx.accum.next_char;
    let accum = &mut ctx.accum;
    gap(accum.text, accum.next_char, accum.runs);
    let after_gap = *ctx.accum.next_char;
    ctx.pen = None;
    walk_nested(
        doc,
        pattern.id,
        pattern.stream,
        &pattern.scope,
        pattern.depth,
        inherited,
        ctx,
    )?;
    let accum = &mut ctx.accum;
    if *accum.next_char == after_gap {
        if after_gap > before {
            accum.text.pop();
            accum.runs.pop();
            *accum.next_char = before;
        }
    } else {
        gap(accum.text, accum.next_char, accum.runs);
    }
    Ok(())
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
    mut inherited: Inherited<'_>,
    ctx: &mut Walk<'_, 'a>,
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
        .and_then(|matrix| doc.resolve_numbers(matrix))
        .map_or(Matrix::IDENTITY, Matrix::from);
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
