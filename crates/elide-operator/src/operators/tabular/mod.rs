//! Tabular-modality operators: structural drops.
//!
//! [`DropRow`] and [`DropColumn`] remove a whole record or field, rather
//! than editing a cell's value, the cell-editing operators live in
//! [`text`](super::text) since a cell is text. Re-exported from
//! [`operators`](super); this module is an internal grouping.

mod drop_column;
mod drop_row;

pub use self::drop_column::DropColumn;
pub use self::drop_row::DropRow;
