//! The content walker: build a page's decoded text and its glyph→bytes
//! [`OffsetMap`] by walking content operators in order, recursing through
//! `Do`-invoked Form XObjects so their text is located too.
//!
//! Walking one glyph at a time is what lets a detected character span map to the
//! exact glyph byte ranges that drew it. The walk mirrors lopdf's own text
//! extraction rule (a large-negative `TJ` kern reads as a space) so the text
//! this produces lines up character-for-character with what a caller runs
//! detection over.

use std::collections::{BTreeMap, BTreeSet};

use elide_core::{Error, ErrorKind, Result};
use lopdf::content::Content;
use lopdf::{Document, Encoding, Object, ObjectId};

use super::glyphs::decode_glyphs;
use super::{Address, FontEntry, GlyphBytes, OffsetMap, OffsetRun, StreamTarget, TextBlock};
use crate::document::Store;

/// Deepest Form-XObject nesting the walker follows, a guard against pathological
/// or cyclic documents (the cycle guard already stops re-entry; this bounds
/// legitimately deep nesting).
const MAX_XOBJECT_DEPTH: u8 = 12;

/// Total Form-XObject walks permitted per page. Depth and the cycle guard bound
/// how *deep* and how re-entrant the walk is, but a form reached through many
/// distinct paths is re-walked on each, so a small document can still fan out to
/// an enormous number of decodes. This caps the total; a genuine page draws far
/// fewer forms than this, so it only bites pathological fan-out.
const MAX_XOBJECT_WALKS: u32 = 4_096;

/// A page's text and glyph map under construction: [`push_glyph`](TextRun::push_glyph)
/// appends the characters a glyph drew (one [`OffsetRun`]), and
/// [`push_gap`](TextRun::push_gap) a synthetic space no glyph drew. The same run
/// accumulates text across the page's own content and the Form XObjects it
/// draws, so a detected span is located wherever it was drawn, each glyph run
/// records the stream its bytes live in.
struct TextRun<'a> {
    text: &'a mut String,
    runs: &'a mut Vec<OffsetRun>,
    /// Next unconsumed decoded-character offset (kept so each run's `chars`
    /// range is filled without recounting `text`).
    next_char: &'a mut usize,
    /// The content stream currently being walked, stamped onto every glyph so a
    /// deletion edits the right stream.
    stream: StreamTarget,
    /// The font-table index of the currently selected font, stamped onto every
    /// glyph pushed so a deletion can later reach that font's `/ToUnicode`.
    font: u16,
}

impl TextRun<'_> {
    /// Append the characters a glyph drew (its decoded text) as one run tied to
    /// the glyph's bytes.
    fn push_glyph(&mut self, text: &str, address: Address, byte_start: usize, byte_end: usize) {
        let start = *self.next_char;
        let count = text.chars().count();
        if count == 0 {
            return;
        }
        self.text.push_str(text);
        *self.next_char += count;
        self.runs.push(OffsetRun {
            chars: start..*self.next_char,
            source: Some(GlyphBytes {
                address,
                byte_start,
                byte_end,
                font: self.font,
            }),
        });
    }

    /// Append a synthetic space (a word gap or `TJ`-array trailing space) that
    /// no glyph drew: one char of gap with no glyph source.
    fn push_gap(&mut self) {
        let start = *self.next_char;
        self.text.push(' ');
        *self.next_char += 1;
        self.runs.push(OffsetRun {
            chars: start..*self.next_char,
            source: None,
        });
    }

    /// Append the text a `Tj`/`TJ` operand list draws, recording each glyph's
    /// bytes. Mirrors lopdf's `collect_text`: a `TJ` array's strings are decoded
    /// in order, and a large-negative kerning number inserts a space (no glyph).
    ///
    /// `operand_base` is the index of `operands[0]` within the operation's full
    /// operand list, so the recorded [`Address`] names the true operand even when
    /// the caller passes a sub-slice (as `"` does, whose string is operand 2). It
    /// is 0 when the whole operand list is passed.
    fn show_text(
        &mut self,
        enc: &Encoding,
        operands: &[Object],
        op: usize,
        operand_base: usize,
    ) -> Result<()> {
        for (i, value) in operands.iter().enumerate() {
            let operand = operand_base + i;
            match value {
                Object::String(bytes, _) => {
                    let address = Address {
                        stream: self.stream,
                        op,
                        operand,
                        item: None,
                    };
                    self.push_glyphs(enc, bytes, address)?;
                }
                Object::Array(arr) => {
                    // A `TJ` array interleaves strings with kerning adjustments (in
                    // thousandths of an em, negated). A large-negative adjustment is
                    // a word gap and reads as a space. The exact threshold is not
                    // font-metric-precise, but it deliberately mirrors lopdf's own
                    // text extraction so the extracted page text, the string a
                    // caller runs detection over, matches character for character;
                    // the glyph map is built in the same pass with the
                    // same rule, so a detected span stays aligned with the glyphs
                    // that drew it. The array is then followed by a trailing space,
                    // also matching lopdf.
                    const WORD_GAP_KERN: f64 = -100.0;
                    for (item_idx, item) in arr.iter().enumerate() {
                        match item {
                            Object::String(bytes, _) => {
                                let address = Address {
                                    stream: self.stream,
                                    op,
                                    operand,
                                    item: Some(item_idx),
                                };
                                self.push_glyphs(enc, bytes, address)?;
                            }
                            Object::Integer(i) if (*i as f64) < WORD_GAP_KERN => self.push_gap(),
                            Object::Real(f) if (*f as f64) < WORD_GAP_KERN => self.push_gap(),
                            _ => {}
                        }
                    }
                    self.push_gap();
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Decode `bytes` into glyphs and append each glyph's characters, tied to its
    /// byte range within the string at `address`.
    fn push_glyphs(&mut self, enc: &Encoding, bytes: &[u8], address: Address) -> Result<()> {
        for glyph in decode_glyphs(enc, bytes)? {
            self.push_glyph(&glyph.text, address, glyph.byte_start, glyph.byte_end);
        }
        Ok(())
    }
}

/// Build one [`TextBlock`] per page by walking its content in operator order,
/// the same order lopdf's text extraction uses, so a character offset maps to
/// the glyph that drew it. Text drawn through a `Do`-invoked Form XObject is
/// walked in place, so it is located and redactable like page-level text.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`](crate::ErrorKind::MalformedInput) if a page's
/// content, resources, or an XObject stream cannot be read;
/// [`ErrorKind::Redaction`](crate::ErrorKind::Redaction) if a page draws text
/// with a font whose encoding cannot be decoded.
pub(crate) fn text_blocks(store: &Store) -> Result<Vec<TextBlock>> {
    let doc = store.doc();
    let max_page_bytes = store.max_page_bytes().get();
    let pages = doc.get_pages();
    let mut out = Vec::with_capacity(pages.len());
    for (&page, &page_id) in &pages {
        out.push(text_block_for_page(doc, page, page_id, max_page_bytes)?);
    }
    Ok(out)
}

/// Build the [`TextBlock`] for one page by walking its content (and the Form
/// XObjects it draws) in operator order, so a character offset maps to the glyph
/// that drew it.
///
/// The per-page primitive [`text_blocks`] loops over. Extraction calls it too,
/// classifying a page it cannot decode as an issue rather than failing the whole
/// document, so its error policy differs from the redactor's fail-closed one.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`](crate::ErrorKind::MalformedInput) if the page's
/// content, resources, or an XObject stream cannot be read;
/// [`ErrorKind::Redaction`](crate::ErrorKind::Redaction) if the page draws text
/// with a font whose encoding cannot be decoded.
pub(crate) fn text_block_for_page(
    doc: &Document,
    page: u32,
    page_id: ObjectId,
    max_page_bytes: usize,
) -> Result<TextBlock> {
    let content_data = doc.get_page_content(page_id);
    // Guard against a decompression bomb: refuse a page whose (already
    // decompressed) content stream exceeds the bound rather than walking it.
    if content_data.len() > max_page_bytes {
        return Err(Error::new(
            ErrorKind::ResourceLimit,
            format!(
                "page {page} content is {} bytes, over the {max_page_bytes}-byte limit",
                content_data.len()
            ),
        ));
    }
    let content = Content::decode(&content_data).map_err(|e| {
        Error::new(
            ErrorKind::MalformedInput,
            format!("page {page} content: {e}"),
        )
    })?;

    // The page's own resources, the scope its `Tf`/`Do` names resolve in.
    let (resources, resource_ids) = doc.get_page_resources(page_id).map_err(|e| {
        Error::new(
            ErrorKind::MalformedInput,
            format!("page {page} resources: {e}"),
        )
    })?;

    let mut text = String::new();
    let mut runs: Vec<OffsetRun> = Vec::new();
    let mut next_char = 0usize;
    let mut fonts: Vec<FontEntry> = Vec::new();
    let mut ctx = Walk {
        accum: TextAccum {
            text: &mut text,
            runs: &mut runs,
            next_char: &mut next_char,
            fonts: &mut fonts,
        },
        active: BTreeSet::new(),
        page,
        max_page_bytes,
        walks_remaining: MAX_XOBJECT_WALKS,
    };

    walk_stream(
        doc,
        &content,
        StreamTarget::PageContent,
        &Scope {
            inline: resources,
            referenced: &resource_ids,
        },
        MAX_XOBJECT_DEPTH,
        None,
        &mut ctx,
    )?;

    Ok(TextBlock {
        page,
        page_id,
        text,
        offsets: OffsetMap::new(runs),
        fonts,
    })
}

/// Walk one content stream in operator order, appending its text and glyph map
/// to `accum`, and recursing into each Form XObject drawn with `Do`.
///
/// `scope` is the resource dictionary a `Tf`/`Do` resolves names in. `stream` is
/// stamped onto every glyph so a deletion edits the correct stream. `active`
/// holds the XObjects currently on the recursion stack (cycle guard); `depth`
/// bounds nesting. Text under an undecodable font fails closed, as at page level.
///
/// `inherited` is the caller's selected font at the `Do` that invoked this
/// stream: a Form XObject inherits the graphics-state font, so text it draws
/// before its own `Tf` is drawn with the caller's font. It is the font-table
/// slot plus the resolved encoding (`&Some` decodable, `&Some(None)` selected
/// but undecodable, `&None`/`None` no font selected). `None` at page level.
fn walk_stream(
    doc: &Document,
    content: &Content,
    stream: StreamTarget,
    scope: &Scope<'_>,
    depth: u8,
    inherited: Option<(u16, &Option<Encoding>)>,
    ctx: &mut Walk<'_>,
) -> Result<()> {
    // Resolve this scope's fonts to encodings, and append a `FontEntry` per font
    // to the shared table. A local name->table-index map points the run at the
    // right entry on each `Tf`.
    let font_dicts = scope_fonts(doc, scope);
    let mut font_slot: BTreeMap<Vec<u8>, u16> = BTreeMap::new();
    let mut encodings: BTreeMap<Vec<u8>, Option<Encoding>> = BTreeMap::new();
    for (name, font) in &font_dicts {
        let to_unicode = font
            .get(b"ToUnicode")
            .ok()
            .and_then(|o| o.as_reference().ok());
        // The font-table index is a `u16` a glyph records to route its code to
        // the right `/ToUnicode` CMap at scrub time. Every stream walk (including
        // each re-walk allowed by the walk budget) appends its scope's fonts, so
        // the table can grow; if it would exceed the `u16` range the slot would
        // truncate and misroute the scrub, leaving deleted text recoverable.
        // Fail closed instead, as everywhere else a glyph might slip the scrub.
        let Ok(slot) = u16::try_from(ctx.accum.fonts.len()) else {
            return Err(Error::new(
                ErrorKind::Redaction,
                format!(
                    "page {} references more than {} fonts across its content \
                     streams; its text cannot be safely redacted",
                    ctx.page,
                    u16::MAX,
                ),
            ));
        };
        ctx.accum.fonts.push(FontEntry { to_unicode });
        font_slot.insert(name.clone(), slot);
        encodings.insert(name.clone(), font.get_font_encoding(doc).ok());
    }

    // The current font's resolved encoding: `None` before any `Tf` (unless a
    // font is inherited from the caller's graphics state, below), or `Some(None)`
    // when the selected font could not be decoded or names a font absent from
    // this scope's resources.
    const UNRESOLVED: &Option<Encoding> = &None;
    // A Form XObject inherits the caller's font, so text it draws before its own
    // `Tf` uses that font; at page level there is nothing to inherit.
    let (mut current, mut font_slot_current): (Option<&Option<Encoding>>, u16) = match inherited {
        Some((slot, enc)) => (Some(enc), slot),
        None => (None, 0),
    };

    for (op_idx, op) in content.operations.iter().enumerate() {
        match op.operator.as_str() {
            "Tf" => {
                if let Some(Object::Name(name)) = op.operands.first() {
                    current = Some(encodings.get(name).unwrap_or(UNRESOLVED));
                    if let Some(&slot) = font_slot.get(name.as_slice()) {
                        font_slot_current = slot;
                    }
                }
            }
            "Tj" | "TJ" | "'" | "\"" => match current {
                Some(Some(enc)) => {
                    let (operands, base): (&[Object], usize) = if op.operator == "\"" {
                        match op.operands.get(2) {
                            Some(s) => (std::slice::from_ref(s), 2),
                            None => (&[], 0),
                        }
                    } else {
                        (&op.operands, 0)
                    };
                    // A short-lived run so its borrow of `accum` ends before the
                    // `Do` arm needs `accum` again.
                    let mut run = ctx.accum.run(stream);
                    run.font = font_slot_current;
                    run.show_text(enc, operands, op_idx, base)?;
                }
                Some(None) => {
                    return Err(Error::new(
                        ErrorKind::Redaction,
                        format!(
                            "page {} draws text with a font whose encoding \
                             could not be decoded or was not found in the page \
                             resources; its text cannot be redacted",
                            ctx.page
                        ),
                    ));
                }
                None => {}
            },
            // A `Do` may draw a Form XObject, whose content draws text in its own
            // stream and resources. Recurse so that text is located and
            // redactable too.
            "Do" => {
                if let Some(Object::Name(name)) = op.operands.first() {
                    // Pass the currently selected font down: the form inherits it
                    // and may draw text before selecting its own.
                    let inherited = current.map(|enc| (font_slot_current, enc));
                    walk_xobject(doc, name, scope, depth, inherited, ctx)?;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// Resolve and recurse into the Form XObject named `name` in `scope`.
///
/// Skips a non-form (e.g. image) XObject, a name absent from the scope, and a
/// re-entrant or too-deep reference (cycle/limit guard). A form's own
/// `/Resources` become the scope for its content; when it has none the parent
/// scope stands in, as the spec allows.
fn walk_xobject(
    doc: &Document,
    name: &[u8],
    scope: &Scope<'_>,
    depth: u8,
    inherited: Option<(u16, &Option<Encoding>)>,
    ctx: &mut Walk<'_>,
) -> Result<()> {
    if depth == 0 {
        return Ok(());
    }
    let Some(xobject_id) = scope_xobject_id(doc, scope, name) else {
        return Ok(());
    };
    // Cycle guard: an XObject already on the stack (or that we cannot read as a
    // stream) is skipped.
    if ctx.active.contains(&xobject_id) {
        return Ok(());
    }
    let Ok(Object::Stream(stream_obj)) = doc.get_object(xobject_id) else {
        return Ok(());
    };
    // Only Form XObjects carry content to walk; images are not text.
    if stream_obj
        .dict
        .get(b"Subtype")
        .and_then(Object::as_name)
        .ok()
        != Some(b"Form")
    {
        return Ok(());
    }
    let bytes = stream_obj
        .decompressed_content()
        .map_err(|e| Error::new(ErrorKind::MalformedInput, format!("XObject content: {e}")))?;
    // Bound the decompressed XObject stream, as for page content: a Form XObject
    // is another place a decompression bomb could hide.
    if bytes.len() > ctx.max_page_bytes {
        return Err(Error::new(
            ErrorKind::ResourceLimit,
            format!(
                "XObject content is {} bytes, over the {}-byte limit",
                bytes.len(),
                ctx.max_page_bytes
            ),
        ));
    }
    let content = Content::decode(&bytes).map_err(|e| {
        Error::new(
            ErrorKind::MalformedInput,
            format!("decode XObject content: {e}"),
        )
    })?;

    // The form's own resources, or the parent scope when it declares none.
    let inner_resources = stream_obj
        .dict
        .get(b"Resources")
        .ok()
        .and_then(|r| dict_or_ref(doc, r));
    let inner_scope = match inner_resources {
        Some(res) => Scope {
            inline: Some(res),
            referenced: &[],
        },
        None => *scope,
    };

    // Charge the total-walk budget for this form (only now that it is a real form
    // we will decode). Exhausting it means a page fans out to more walks than any
    // legitimate document, so fail closed rather than let the walk run away.
    let Some(remaining) = ctx.walks_remaining.checked_sub(1) else {
        return Err(Error::new(
            ErrorKind::ResourceLimit,
            format!(
                "page {} draws more than {MAX_XOBJECT_WALKS} Form XObjects; \
                 its text cannot be walked within the resource budget",
                ctx.page
            ),
        ));
    };
    ctx.walks_remaining = remaining;

    ctx.active.insert(xobject_id);
    let result = walk_stream(
        doc,
        &content,
        StreamTarget::XObject(xobject_id),
        &inner_scope,
        depth - 1,
        inherited,
        ctx,
    );
    ctx.active.remove(&xobject_id);
    result
}

/// Collect the `/Font` entries of a resource scope (inline plus any referenced
/// resource dictionaries), name -> font dictionary.
fn scope_fonts<'a>(
    doc: &'a Document,
    scope: &Scope<'a>,
) -> BTreeMap<Vec<u8>, &'a lopdf::Dictionary> {
    let mut out = BTreeMap::new();
    let mut add = |resources: &'a lopdf::Dictionary| {
        if let Ok(font) = resources.get(b"Font")
            && let Some(font_dict) = dict_or_ref(doc, font)
        {
            for (name, value) in font_dict.iter() {
                let font = match value {
                    Object::Reference(id) => doc.get_dictionary(*id).ok(),
                    Object::Dictionary(dict) => Some(dict),
                    _ => None,
                };
                if let Some(font) = font {
                    out.entry(name.clone()).or_insert(font);
                }
            }
        }
    };
    if let Some(inline) = scope.inline {
        add(inline);
    }
    for &id in scope.referenced {
        if let Ok(resources) = doc.get_dictionary(id) {
            add(resources);
        }
    }
    out
}

/// Resolve the object id of the XObject named `name` in `scope`'s `/XObject`
/// dictionary, if it is an indirect reference.
fn scope_xobject_id(doc: &Document, scope: &Scope<'_>, name: &[u8]) -> Option<ObjectId> {
    let resource_dicts = scope.inline.into_iter().chain(
        scope
            .referenced
            .iter()
            .filter_map(|&id| doc.get_dictionary(id).ok()),
    );
    for resources in resource_dicts {
        if let Ok(xobjects) = resources.get(b"XObject")
            && let Some(xobjects) = dict_or_ref(doc, xobjects)
            && let Ok(entry) = xobjects.get(name)
            && let Ok(id) = entry.as_reference()
        {
            return Some(id);
        }
    }
    None
}

/// Resolve an object that is a dictionary, inline or by reference.
fn dict_or_ref<'a>(doc: &'a Document, object: &'a Object) -> Option<&'a lopdf::Dictionary> {
    match object {
        Object::Dictionary(dict) => Some(dict),
        Object::Reference(id) => doc.get_dictionary(*id).ok(),
        _ => None,
    }
}

/// A resource dictionary scope a stream's `Tf`/`Do` names resolve in: an inline
/// dictionary and any referenced resource dictionaries (a page can inherit
/// several through its tree).
#[derive(Clone, Copy)]
struct Scope<'a> {
    inline: Option<&'a lopdf::Dictionary>,
    referenced: &'a [ObjectId],
}

/// The growing text, glyph map, and font table a walk appends to. Borrowed so a
/// single accumulation spans the page content and every Form XObject it draws.
struct TextAccum<'a> {
    text: &'a mut String,
    runs: &'a mut Vec<OffsetRun>,
    next_char: &'a mut usize,
    fonts: &'a mut Vec<FontEntry>,
}

impl TextAccum<'_> {
    /// Borrow the accumulator as a [`TextRun`] stamping glyphs into `stream`,
    /// starting on font slot 0.
    fn run(&mut self, stream: StreamTarget) -> TextRun<'_> {
        TextRun {
            text: self.text,
            runs: self.runs,
            next_char: self.next_char,
            stream,
            font: 0,
        }
    }
}

/// The mutable state threaded through a page's content walk, unchanged as it
/// recurses into Form XObjects: the accumulating text/glyph/font output, the
/// XObjects currently on the recursion stack (cycle guard), the 1-based page
/// number for diagnostics, the per-stream decompression-bomb bound, and the
/// remaining total-walk budget.
struct Walk<'a> {
    accum: TextAccum<'a>,
    active: BTreeSet<ObjectId>,
    page: u32,
    /// Maximum decompressed size of any single content stream (page or XObject),
    /// refusing a decompression bomb.
    max_page_bytes: usize,
    /// Form XObject walks still permitted on this page. The stack-based `active`
    /// guard stops re-entry (cycles) but not re-walks: a form reached through
    /// several distinct paths is walked once per path, so a shallow tree that
    /// draws each child many times per level can fan out to a huge number of
    /// stream decodes without ever recursing cyclically. This bounds the total,
    /// independent of nesting depth.
    walks_remaining: u32,
}
