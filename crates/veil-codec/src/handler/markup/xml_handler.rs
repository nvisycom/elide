//! XML handler side: the [`XmlHandler`] type, its [`Format`] descriptor,
//! and the [`XmlEncoder`] that re-serializes a mutated
//! [`RedactableItem`](super::RedactableItem) stream back into XML.
//!
//! Unlike the HTML encoder (which rebuilds a DOM), the XML encoder
//! preserves the document **verbatim**: it splices each item's current
//! value back at its recorded source byte span into the retained raw
//! string, leaving the declaration, whitespace, attribute quoting, and
//! everything outside the redacted spans byte-identical. Splices apply
//! right-to-left so an earlier edit's length delta never shifts a later
//! span.

use std::ops::Range;

use veil_core::modality::text::Text;
use veil_core::{Error, ErrorKind};

use super::{MarkupEncoder, MarkupHandler, RedactableItem};
use crate::content::ContentData;
use crate::{Format, FormatId};

/// Stable [`FormatId`] for the XML codec.
pub const FORMAT_ID: FormatId = FormatId::from_static("veil.text.xml");

/// Handler type for loaded XML content.
pub type XmlHandler = MarkupHandler<XmlEncoder>;

/// An XML [`RedactableItem`](super::RedactableItem) addressed by the
/// source byte span its `value` occupies in the original document.
pub(super) type XmlItem = RedactableItem<XmlSpan>;

/// The source byte span (in the retained raw document) that a
/// [`RedactableItem`](super::RedactableItem)'s value occupies — the
/// region the encoder overwrites. These are the *inner* bytes: a text
/// node's text, an attribute value between the quotes, a comment body
/// between `<!--` and `-->`, a CDATA payload between `<![CDATA[` and
/// `]]>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XmlSpan(pub(super) Range<usize>);

/// [`Format`] descriptor registered into [`crate::CodecRegistry`].
pub fn format() -> Format {
    Format::new::<Text, _>(FORMAT_ID.clone(), super::XmlLoader)
        .with_extensions(["xml"])
        .with_content_types(["application/xml", "text/xml"])
}

/// Re-serializes a mutated item stream by splicing each value back at its
/// source span into the retained raw document.
#[derive(Debug)]
pub struct XmlEncoder {
    pub(super) raw: String,
}

impl MarkupEncoder for XmlEncoder {
    type Address = XmlSpan;

    fn encode(&self, items: &[XmlItem]) -> Result<ContentData, Error> {
        // Item spans are recorded by the loader from disjoint quick-xml
        // events over this same `raw`, so they never overlap. Applying
        // them right-to-left means each splice's length delta can't shift
        // the spans of items earlier in the document.
        let mut ordered: Vec<&XmlItem> = items.iter().collect();
        ordered.sort_by_key(|item| std::cmp::Reverse(item.address.0.start));

        let mut out = self.raw.clone();
        for item in ordered {
            let Range { start, end } = item.address.0.clone();
            // Spans index into `out`, which starts as `raw` and only ever
            // grows/shrinks to the right of the current splice, so they
            // stay in-bounds and on char boundaries by construction. The
            // guards are defensive — a malformed loader would surface here
            // rather than panic in `replace_range`.
            if end > out.len() || start > end {
                return Err(Error::new(
                    ErrorKind::Validation,
                    format!("xml splice span {start}..{end} out of bounds (len {})", out.len()),
                ));
            }
            if !out.is_char_boundary(start) || !out.is_char_boundary(end) {
                return Err(Error::new(
                    ErrorKind::Validation,
                    format!("xml splice span {start}..{end} falls mid-character"),
                ));
            }
            // `value` is the raw on-the-wire slice (the loader stores
            // source bytes verbatim, never a decoded form), so it splices
            // back with no escape transform — only the redacted sub-range
            // ever changed.
            out.replace_range(start..end, &item.value);
        }
        Ok(ContentData::new(out.into_bytes().into()))
    }
}
