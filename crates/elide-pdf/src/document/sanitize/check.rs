//! The output check: an encoded PDF is refused when sanitizing it again would
//! still remove something.

use elide_core::{Error, Result};
use lopdf::Document;

use super::SanitizeDocumentExt;

/// Check that `doc` carries no hidden content: sanitize a copy, and refuse the
/// document naming whatever that removed.
///
/// # Errors
///
/// [`ErrorKind::Sanitization`](elide_core::ErrorKind::Sanitization) naming what
/// survived.
pub(super) fn check(doc: &Document) -> Result<()> {
    let survived = doc.clone().sanitize()?;
    if survived.is_empty() {
        return Ok(());
    }
    let items: Vec<String> = survived
        .iter()
        .map(|entry| format!("{:?}", entry.content))
        .collect();
    Err(Error::sanitization(format!(
        "encoded PDF still carries hidden content: {}",
        items.join(", ")
    )))
}
