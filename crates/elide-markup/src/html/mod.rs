//! The HTML format: the shared markup engine read as a browser reads HTML.

mod handler;
mod loader;
pub(crate) mod meta;

pub use self::handler::format;
pub(crate) use self::loader::HtmlLoader;
