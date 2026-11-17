//! The output check: an encoded document is a rebuild, so rebuilding it
//! again drops nothing.

use elide_core::{Error, Result};
use lopdf::Document;

use super::Builder;
use crate::pdf::{Pdf, Read};

/// Whether the document is a rebuild: rebuilding it again drops nothing.
/// What the codec runs on an encoded PDF before returning it.
///
/// # Errors
///
/// [`ErrorKind::Sanitization`](elide_core::ErrorKind::Sanitization) naming what
/// a rebuild would drop, or as [`Rebuild`](super::Rebuild).
pub(crate) struct Check;

impl Read for Check {
    type Output = ();

    fn read(self, pdf: &Pdf) -> Result<Self::Output> {
        check(pdf.doc())
    }
}

/// The [`Check`] of `doc`.
pub(super) fn check(doc: &Document) -> Result<()> {
    let mut rebuild = Builder::new(doc);
    rebuild.run()?;
    if rebuild.stripped.is_empty() && rebuild.residue == 0 {
        return Ok(());
    }
    let mut items: Vec<String> = rebuild
        .stripped
        .iter()
        .map(|entry| format!("{:?}", entry.content))
        .collect();
    if rebuild.residue > 0 {
        items.push(format!("{} unmodelled entries", rebuild.residue));
    }
    Err(Error::sanitization(format!(
        "encoded PDF still carries content a rebuild drops: {}",
        items.join(", ")
    )))
}
