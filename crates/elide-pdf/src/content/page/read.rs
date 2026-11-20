//! Rendering a PDF's pages as the images that replace them: each page
//! redacted from its render, rendered fresh with the regions redacted on it
//! painted over.

use std::collections::BTreeSet;

use elide_core::{Error, Result};

use super::{PageReplacement, ScannedPage};
use crate::pdf::Pdf;
use crate::pdf::render::each_page;

/// The `scanned` pages rendered fresh at `scale`, each with the regions
/// redacted on it painted over; see [`Repaint`](super::Repaint).
pub(super) fn repaint(
    pdf: &Pdf,
    scanned: &[ScannedPage],
    scale: f32,
) -> Result<Vec<PageReplacement>> {
    let numbers: BTreeSet<u32> = scanned.iter().map(|page| page.number).collect();
    let mut out = Vec::new();
    each_page(pdf, &numbers, scale, |number, image| {
        let page = scanned
            .iter()
            .find(|page| page.number == number)
            .ok_or_else(|| Error::redaction(format!("page {number} was not asked for")))?;
        out.push(page.repainted(image)?);
        Ok(())
    })?;
    Ok(out)
}
