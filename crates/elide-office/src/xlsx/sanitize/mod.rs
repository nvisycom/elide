//! The SpreadsheetML element edits: what hides inside the parts a workbook
//! keeps.
//!
//! - **Editor fingerprints**, always: the revision ids Excel stamps on the
//!   workbook, sheets, and tables, the absolute path the workbook was last
//!   saved to, and the file-sharing reservation (a user name and a password
//!   hash).
//! - **External references**, always: the workbook's list of linked external
//!   workbooks, whose parts are removed.
//! - **Comment notes**, under [`KeepOnlyEssential`](elide_core::redaction::Disposition::KeepOnlyEssential): the legacy VML shape
//!   that draws each comment's note, left behind by the comments part. Under
//!   [`FlagEverything`](elide_core::redaction::Disposition::FlagEverything) the
//!   shape stays, for the comment it draws, without any text of its own.
//! - **Threaded-comment dates and mentions**, when comments are redacted: when
//!   a comment was written, and its mentions, which point into the comment's
//!   text by offset and would point at the wrong text once it is redacted (the
//!   mentioned name stays in the text and is redacted with it).

use std::ops::Range;

use elide_core::Result;
use elide_core::redaction::{Disposition, HiddenContent, HiddenContentPolicy};
use quick_xml::events::Event;

use super::SheetFormat;
use super::names::{
    DEFINED_NAME, HYPERLINK_LOCATION, SHEET_NAME, TABLE_COLUMN_NAME, TABLE_DISPLAY_NAME, TABLE_NAME,
};
use crate::opc::sanitize::{Edits, Sanitizer};
use crate::opc::xml::edit::{Cut, Declared, Edit, Edited, Local, SpannedReader};
use crate::opc::xml::ns::{THREADED, VML, VML_EXCEL, X, X15AC, XR, XR2, XR3, XR6, XR10};
use crate::opc::{
    Collaboration, HiddenAttributes, Ns, PartKind, TextAttribute, attribute_value, malformed_xml,
};

/// The revision ids Excel stamps on the workbook, sheets, and tables.
const EDITING_SESSION_IDS: Declared =
    Declared::removing(HiddenContent::EditingSessionId, &[(XR, "revisionPtr")]).with_attributes(&[
        (XR, Local::Any),
        (XR2, Local::Any),
        (XR3, Local::Any),
        (XR6, Local::Any),
        (XR10, Local::Any),
    ]);

/// The absolute path the workbook was last saved to.
const FILE_PATH: Declared = Declared::removing(HiddenContent::FilePath, &[(X15AC, "absPath")]);

/// The file-sharing reservation: a user name and a password hash.
const CREDENTIALS: Declared = Declared::removing(HiddenContent::Credential, &[(X, "fileSharing")]);

/// The workbook's list of linked external workbooks.
const EXTERNAL_WORKBOOKS: Declared =
    Declared::removing(HiddenContent::Link, &[(X, "externalReferences")]);

/// When a threaded comment was written.
const COMMENT_DATES: HiddenAttributes = HiddenAttributes::strip(
    HiddenContent::Comment,
    &[TextAttribute::new(
        (Ns::In(THREADED), Local::Exact("threadedComment")),
        (Ns::Unqualified, "dT"),
    )],
);

/// A threaded comment's mentions.
const MENTIONS: Declared = Declared::removing(HiddenContent::Comment, &[(THREADED, "mentions")]);

/// The VML shapes that draw comment notes: a `v:shape` whose `x:ClientData`
/// has `ObjectType="Note"`. A shape's kind is in a child, so this edit reads
/// each shape whole before cutting it.
struct CommentNotes;

/// The text in the text boxes of the [comment notes](CommentNotes), which
/// leaves the shapes in place.
struct CommentNoteText;

/// An open VML shape as [`cut_notes`] reads it.
struct OpenShape {
    start: usize,
    note: bool,
    /// The non-blank text of its text box.
    text: Vec<Range<usize>>,
}

impl Edit for CommentNotes {
    fn content(&self) -> Option<HiddenContent> {
        Some(HiddenContent::Comment)
    }

    fn apply(&self, raw: &str) -> Result<Option<Edited>> {
        cut_notes(raw, NoteCut::Shape)
    }
}

impl Edit for CommentNoteText {
    fn content(&self) -> Option<HiddenContent> {
        Some(HiddenContent::Comment)
    }

    fn apply(&self, raw: &str) -> Result<Option<Edited>> {
        cut_notes(raw, NoteCut::Text)
    }
}

/// What [`cut_notes`] cuts of a comment note.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NoteCut {
    /// The whole shape.
    Shape,
    /// The text of its text box.
    Text,
}

/// What an open element of a VML drawing is, as [`cut_notes`] reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Opened {
    Shape,
    TextBox,
    Other,
}

/// `raw` without its comment notes, each cut as `cut` says. The count is of
/// the notes changed.
fn cut_notes(raw: &str, cut: NoteCut) -> Result<Option<Edited>> {
    let mut reader = SpannedReader::new_ns(raw);
    let mut shapes: Vec<OpenShape> = Vec::new();
    // What each open element is.
    let mut open: Vec<Opened> = Vec::new();
    let mut cuts: Vec<Range<usize>> = Vec::new();
    let mut count = 0;
    loop {
        let (namespace, event, span) = reader.read_resolved_event().map_err(malformed_xml)?;
        match event {
            Event::Eof => break,
            Event::Start(ref e) | Event::Empty(ref e) => {
                let local = e.local_name();
                let local = local.into_inner();
                let in_ns = |ns: &str| namespace.as_deref() == Some(ns);
                if in_ns(VML_EXCEL) && local == "ClientData" {
                    // The kind as XML reads it, its references decoded; one
                    // that does not decode may be a note.
                    let mut note = false;
                    for attr in e.attributes() {
                        let attr = attr.map_err(malformed_xml)?;
                        note |= attr.key.as_ref() == "ObjectType"
                            && attribute_value(&attr.value).map_or(true, |kind| kind == "Note");
                    }
                    if note && let Some(shape) = shapes.last_mut() {
                        shape.note = true;
                    }
                }
                if matches!(event, Event::Start(_)) {
                    let shape = in_ns(VML) && local == "shape";
                    if shape {
                        shapes.push(OpenShape {
                            start: span.start,
                            note: false,
                            text: Vec::new(),
                        });
                    }
                    open.push(if shape {
                        Opened::Shape
                    } else if in_ns(VML) && local == "textbox" {
                        Opened::TextBox
                    } else {
                        Opened::Other
                    });
                }
            }
            Event::Text(_) | Event::GeneralRef(_) | Event::CData(_)
                if open.contains(&Opened::TextBox) && !raw[span.clone()].trim().is_empty() =>
            {
                if let Some(shape) = shapes.last_mut() {
                    shape.text.push(span);
                }
            }
            Event::End(_) => {
                if open.pop() == Some(Opened::Shape)
                    && let Some(shape) = shapes.pop()
                    && shape.note
                {
                    match cut {
                        NoteCut::Shape => {
                            count += 1;
                            cuts.push(shape.start..span.end);
                        }
                        NoteCut::Text if !shape.text.is_empty() => {
                            count += 1;
                            cuts.extend(shape.text);
                        }
                        NoteCut::Text => {}
                    }
                }
            }
            _ => {}
        }
    }
    // The cuts may nest: a crafted drawing can put a shape inside another.
    let cuts: Vec<Cut<&str>> = cuts.into_iter().map(Cut::remove).collect();
    Ok(Edited::from_cuts(raw, cuts, count))
}

/// The text attributes a workbook surfaces for detection: its sheet, defined,
/// table, and table column names and the hyperlink locations that refer to
/// them (all fitted to Excel's rules when redacted, see
/// [`names`](super::names)), a hyperlink's display text, a data validation's
/// prompt and error message, a scenario's name, comment, and author, a custom
/// view's name (often its owner's), and a threaded-comment author's name and
/// account.
pub(super) const TEXT_ATTRIBUTES: &[TextAttribute] = &[
    SHEET_NAME,
    DEFINED_NAME,
    TABLE_NAME,
    TABLE_DISPLAY_NAME,
    TABLE_COLUMN_NAME,
    HYPERLINK_LOCATION,
    TextAttribute::unqualified("hyperlink", "display"),
    TextAttribute::unqualified("dataValidation", "promptTitle"),
    TextAttribute::unqualified("dataValidation", "prompt"),
    TextAttribute::unqualified("dataValidation", "errorTitle"),
    TextAttribute::unqualified("dataValidation", "error"),
    TextAttribute::unqualified("scenario", "name"),
    TextAttribute::unqualified("scenario", "comment"),
    TextAttribute::unqualified("scenario", "user").of(Collaboration::Author),
    TextAttribute::unqualified("customWorkbookView", "name"),
    TextAttribute::unqualified("person", "displayName").of(Collaboration::Author),
    TextAttribute::unqualified("person", "userId").of(Collaboration::Author),
];

impl Sanitizer for SheetFormat {
    fn edits(&self, kind: PartKind, policy: HiddenContentPolicy) -> Edits<'static> {
        match kind {
            PartKind::Comments => vec![&EDITING_SESSION_IDS, &COMMENT_DATES, &MENTIONS],
            PartKind::Drawing if !policy.collaboration.treats_as_content() => {
                vec![&EDITING_SESSION_IDS, &CommentNotes]
            }
            PartKind::Drawing if policy.collaboration == Disposition::FlagEverything => {
                vec![&EDITING_SESSION_IDS, &CommentNoteText]
            }
            PartKind::Main => vec![
                &EDITING_SESSION_IDS,
                &FILE_PATH,
                &CREDENTIALS,
                &EXTERNAL_WORKBOOKS,
            ],
            PartKind::Story | PartKind::Structure | PartKind::Drawing => {
                vec![&EDITING_SESSION_IDS]
            }
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests;
