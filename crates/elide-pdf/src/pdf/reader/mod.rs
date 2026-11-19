//! Reading a PDF: hayro-syntax's objects extended with the reads the crate
//! shares, the resource scope a content stream's names resolve in, the name
//! trees under the catalog's `/Names`, page geometry, and fonts.

pub(crate) mod font;
mod matrix;
pub(crate) mod name_tree;
mod object;
mod page;
mod scope;

pub(crate) use self::matrix::Matrix;
pub(crate) use self::name_tree::{NameTree, Node, Pair};
pub(crate) use self::object::{
    DictExt, MAX_CONTENT_BYTES, ObjectId, StreamExt, id, numbers, page_content, page_stream_ids,
    page_streams, reference_to, text_string,
};
pub(crate) use self::page::{
    DeviceRect, GlyphRect, PRINT_BOXES, PageBoxes, PageFrame, PageGeometry, normalize_rect,
    page_boxes, page_frame,
};
pub(crate) use self::scope::Scope;
