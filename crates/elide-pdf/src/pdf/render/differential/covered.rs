//! Covered text against PDFium: a covered span's glyphs are deleted, and
//! every other character stays exactly where PDFium drew it before.

use elide_core::primitive::Color;

use super::{Placed, Point, same, theirs};
use crate::content::text::{Detection, Text, TextRedaction};
use crate::fixtures::{DocBuilder, Page, type1};
use crate::pdf::Pdf;
use crate::sanitize::Rebuild;

/// PDFium draws every character a cover leaves where it drew it before,
/// through character and word spacing, horizontal scaling, a `TJ` array's
/// kerns and a line `'` shows; none of the covered text is left.
#[test]
#[ignore = "requires the native PDFium library at runtime"]
fn covered_text_stays_where_pdfium_drew_it() {
    let mut pdf = DocBuilder::with_version("1.5");
    let font = pdf.add(type1("Helvetica"));
    pdf.page(
        Page::new()
            .font("F1", font)
            .content(
                b"BT /F1 12 Tf 1.5 Tc 4 Tw 110 Tz 14 TL 10 60 Td \
                  [(Call ) -200 (Jane Roe) 150 ( now)] TJ (Ann Lee here) ' ET"
                    .to_vec(),
            )
            .size(300, 100),
    );
    let source = Pdf::open(&pdf.bytes()).unwrap().read(Rebuild).unwrap().pdf;
    let blocks = source.read(Text).unwrap();
    let text = &blocks[0].text;
    let span = |value: &str| {
        let start = text[..text.find(value).unwrap()].chars().count();
        Detection::new(1, start, start + value.chars().count()).covering(Color::BLACK)
    };
    let mut edit = source.edit();
    edit.redact(TextRedaction {
        source: &source,
        blocks: &blocks,
        detections: &[span("Jane Roe"), span("Lee")],
    })
    .unwrap();
    let out = edit.finish().unwrap();

    let before = &theirs(&source).unwrap()[0];
    let after = &theirs(&out).unwrap()[0];
    // PDFium drops a space that follows a wide gap, as the one after each
    // cover is, so only the characters that draw ink are compared.
    let inked = |chars: &[Placed<char>]| -> Vec<(char, Point)> {
        chars
            .iter()
            .filter(|c| !c.text.is_whitespace())
            .map(|c| (c.text, c.origin))
            .collect()
    };
    // The covered characters are PDFium's own of the two spans, by their
    // place among the inked characters of the text the reader read.
    let inked_text: Vec<char> = text.chars().filter(|c| !c.is_whitespace()).collect();
    let covered_at = |value: &str| {
        let value: Vec<char> = value.chars().filter(|c| !c.is_whitespace()).collect();
        let start = inked_text
            .windows(value.len())
            .position(|window| window == value.as_slice())
            .unwrap();
        start..start + value.len()
    };
    assert_eq!(
        inked(before).iter().map(|(c, _)| *c).collect::<Vec<_>>(),
        inked_text
    );
    let (jane, lee) = (covered_at("Jane Roe"), covered_at("Lee"));
    let expected: Vec<_> = inked(before)
        .into_iter()
        .enumerate()
        .filter(|(i, _)| !jane.contains(i) && !lee.contains(i))
        .map(|(_, c)| c)
        .collect();
    let left = inked(after);
    assert_eq!(left.len(), expected.len(), "{after:?}");
    for ((c, origin), (want, was)) in left.iter().zip(&expected) {
        assert!(
            c == want && same(*origin, *was),
            "{c:?} at {origin:?}, where {want:?} was at {was:?}"
        );
    }
}
