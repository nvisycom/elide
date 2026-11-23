//! Reading a PDF: lopdf's object model extended with the reads the crate
//! shares, the resource scope a content stream's names resolve in, the name
//! trees under the catalog's `/Names`, page geometry, and fonts.

pub(crate) mod font;
mod matrix;
pub(crate) mod name_tree;
mod object;
mod page;
mod point;
mod scope;

pub(crate) use self::matrix::Matrix;
pub(crate) use self::name_tree::{NameTree, Node};
pub(crate) use self::object::{
    DictionaryExt, DocumentExt, MAX_CONTENT_BYTES, ObjectExt, StreamExt, parse_content,
};
pub(crate) use self::page::{
    DeviceRect, GlyphRect, PRINT_BOXES, PageBoxes, PageFrame, PageGeometry, normalize_rect,
    page_boxes, page_frame,
};
pub(crate) use self::point::{Point, Vector};
pub(crate) use self::scope::Scope;
