//! The XML format: strict, well-formed XML over the shared markup engine.

mod handler;
mod loader;

#[cfg(test)]
pub(crate) use self::handler::FORMAT_ID;
pub use self::handler::format;
pub(crate) use self::loader::XmlLoader;
