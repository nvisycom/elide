//! The XML format: strict, well-formed XML over the shared markup engine.

mod handler;
mod loader;

pub use self::handler::format;
pub(crate) use self::loader::XmlLoader;
