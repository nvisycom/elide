//! Covered text against PDFium: a covered span's glyphs are deleted, and
//! every other character stays exactly where PDFium drew it before.

use elide_core::primitive::Color;

use super::{Placed, same, theirs};
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
    // cover is, so only the characters that draw ink are counted.
    let inked = |chars: &[Placed<char>]| chars.iter().filter(|c| !c.text.is_whitespace()).count();
    let covered = "JaneRoe".len() + "Lee".len();
    assert_eq!(inked(after), inked(before) - covered, "{after:?}");
    for c in after {
        assert!(
            before
                .iter()
                .any(|was| was.text == c.text && same(was.origin, c.origin)),
            "{:?} at {:?} moved",
            c.text,
            c.origin
        );
    }
    assert!(!after.iter().any(|c| matches!(c.text, 'J' | 'L')));
}
