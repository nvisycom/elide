//! [`DataWriter`] trait: applying a batch of replacements.

use std::future::Future;

use super::Modality;
use crate::error::Result;
use crate::operator::Redactions;

/// Applies a [`Redactions`] batch back into some target.
///
/// The write counterpart to [`DataReader`]: implemented by a modality's
/// mutable content holder (a text buffer being rewritten, an image being
/// painted over), it takes the `(location, replacement)` pairs an
/// anonymizer produced and applies them into the document. This
/// completes the redaction round-trip (read the values, compute
/// replacements, write them) while keeping operators free of format
/// knowledge: the writer owns the *how* of applying each modality's
/// replacements.
///
/// The writer owns the *ordering*, too: it decides the order that keeps
/// offsets valid for its format (e.g. back-to-front for text, so a
/// length change doesn't shift later locations;
/// [`Redactions::sort_by_position`] gives the document-order baseline to
/// reverse). The first failure aborts the batch.
///
/// [`DataReader`]: super::DataReader
pub trait DataWriter<M: Modality>: Send + Sync {
    /// Apply every `(location, replacement)` pair in `redactions`.
    fn write_at(&mut self, redactions: Redactions<M>) -> impl Future<Output = Result<()>> + Send;
}
