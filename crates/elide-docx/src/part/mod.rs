//! The typed identity and classification of a DOCX package part.

mod kind;
mod path;

pub use self::kind::{EmbeddingKind, PartKind};
pub use self::path::PartPath;
