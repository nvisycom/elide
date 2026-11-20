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

use super::SheetClassifier;
use super::names::{
    DEFINED_NAME, HYPERLINK_LOCATION, SHEET_NAME, TABLE_COLUMN_NAME, TABLE_DISPLAY_NAME, TABLE_NAME,
};
use crate::opc::ns::{VML, VML_EXCEL, X};
use crate::opc::sanitize::{Edits, Sanitizer};
use crate::opc::xml_edit::{Declared, Edit, Edited, Local, SpannedReader};
use crate::opc::{
    Collaboration, HiddenAttributes, Ns, PartKind, TextAttribute, attribute_value, malformed_xml,
};

/// The Excel 2010 application-compatibility namespace (the saved-to path).
const X15AC: &str = "http://schemas.microsoft.com/office/spreadsheetml/2010/11/ac";
/// The Excel revision-id namespaces.
const XR: &str = "http://schemas.microsoft.com/office/spreadsheetml/2014/revision";
const XR2: &str = "http://schemas.microsoft.com/office/spreadsheetml/2015/revision2";
const XR3: &str = "http://schemas.microsoft.com/office/spreadsheetml/2016/revision3";
const XR6: &str = "http://schemas.microsoft.com/office/spreadsheetml/2016/revision6";
const XR10: &str = "http://schemas.microsoft.com/office/spreadsheetml/2016/revision10";

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

/// The threaded-comments namespace.
const THREADED: &str = "http://schemas.microsoft.com/office/spreadsheetml/2018/threadedcomments";

/// When a threaded comment was written.
const COMMENT_DATES: HiddenAttributes = HiddenAttributes::strip(
    HiddenContent::Comment,
    &[TextAttribute::new(
        (Ns::In(THREADED), Local::Exact("threadedComment")),
        (Ns::None, "dT"),
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
        cut_notes(raw, true)
    }
}

impl Edit for CommentNoteText {
    fn content(&self) -> Option<HiddenContent> {
        Some(HiddenContent::Comment)
    }

    fn apply(&self, raw: &str) -> Result<Option<Edited>> {
        cut_notes(raw, false)
    }
}

/// `raw` without its comment notes: each `whole` shape, or else the text of each
/// one's text box. The count is of the notes changed.
fn cut_notes(raw: &str, whole: bool) -> Result<Option<Edited>> {
    let mut reader = SpannedReader::new_ns(raw);
    let mut shapes: Vec<OpenShape> = Vec::new();
    // What each open element is.
    let mut open: Vec<(bool, bool)> = Vec::new();
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
                    let note = e.attributes().flatten().any(|attr| {
                        attr.key.local_name().as_ref() == "ObjectType"
                            && attribute_value(&attr.value).map_or(true, |kind| kind == "Note")
                    });
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
                    open.push((shape, in_ns(VML) && local == "textbox"));
                }
            }
            Event::Text(_) | Event::GeneralRef(_) | Event::CData(_)
                if open.iter().any(|&(_, textbox)| textbox)
                    && !raw[span.clone()].trim().is_empty() =>
            {
                if let Some(shape) = shapes.last_mut() {
                    shape.text.push(span);
                }
            }
            Event::End(_) => {
                if open.pop().is_some_and(|(shape, _)| shape)
                    && let Some(shape) = shapes.pop()
                    && shape.note
                {
                    if whole {
                        count += 1;
                        cuts.push(shape.start..span.end);
                    } else if !shape.text.is_empty() {
                        count += 1;
                        cuts.extend(shape.text);
                    }
                }
            }
            _ => {}
        }
    }
    // The cuts may nest: a crafted drawing can put a shape inside another.
    let cuts = cuts.into_iter().map(|cut| (cut, "")).collect();
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

impl Sanitizer for SheetClassifier {
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
mod tests {
    use elide_core::redaction::{Disposition, HiddenContent, HiddenContentPolicy};

    use super::*;
    use crate::fixtures::{pack_parts, part_names, read_text_part};
    use crate::opc::sanitize::sanitize;
    use crate::opc::xml_edit::expect_edit;

    /// A workbook with a revision log and a legacy drawing holding one comment
    /// note and one button.
    fn workbook() -> Vec<u8> {
        let types = r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="vml" ContentType="application/vnd.openxmlformats-officedocument.vmlDrawing"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/revisions/revisionHeaders.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.revisionHeaders+xml"/></Types>"#;
        let workbook = format!(r#"<?xml version="1.0"?><workbook xmlns="{X}"/>"#);
        let revisions = format!(
            r#"<?xml version="1.0"?><headers xmlns="{X}"><header userName="Jane Roe"/></headers>"#
        );
        let shape = |kind: &str| {
            format!(
                r##"<v:shape id="s{kind}" type="#_x0000_t202"><v:textbox><div>{kind}</div></v:textbox><x:ClientData ObjectType="{kind}"><x:Row>0</x:Row></x:ClientData></v:shape>"##
            )
        };
        let vml = format!(
            r#"<xml xmlns:v="urn:schemas-microsoft-com:vml" xmlns:x="urn:schemas-microsoft-com:office:excel">{}{}</xml>"#,
            shape("Note"),
            shape("Button")
        );
        pack_parts(&[
            ("[Content_Types].xml", types.as_bytes()),
            ("xl/workbook.xml", workbook.as_bytes()),
            ("xl/revisions/revisionHeaders.xml", revisions.as_bytes()),
            ("xl/drawings/vmlDrawing1.vml", vml.as_bytes()),
        ])
    }

    fn count(stripped: &elide_codec::StrippedContent, content: HiddenContent) -> usize {
        stripped
            .iter()
            .filter(|entry| entry.content == content)
            .map(|entry| entry.count)
            .sum()
    }

    #[test]
    fn threaded_comments_go_under_every_policy() {
        let types = r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/threadedComments/threadedComment1.xml" ContentType="application/vnd.ms-excel.threadedcomments+xml"/><Override PartName="/xl/persons/person.xml" ContentType="application/vnd.ms-excel.person+xml"/></Types>"#;
        let workbook = format!(r#"<?xml version="1.0"?><workbook xmlns="{X}"/>"#);
        let comments = format!(
            r#"<?xml version="1.0"?><ThreadedComments xmlns="{THREADED}"><threadedComment ref="A1" dT="2024-01-02T03:04:05.00" personId="{{P}}" id="{{C}}"><text>@Jane Roe please check</text><mentions><mention mentionpersonId="{{P}}" mentionId="{{M}}" startIndex="0" length="9"/></mentions></threadedComment></ThreadedComments>"#
        );
        let persons = format!(
            r#"<?xml version="1.0"?><personList xmlns="{THREADED}"><person displayName="Jane Roe" id="{{P}}" userId="jane@example.com" providerId="PeoplePicker"/></personList>"#
        );
        let package = pack_parts(&[
            ("[Content_Types].xml", types.as_bytes()),
            ("xl/workbook.xml", workbook.as_bytes()),
            (
                "xl/threadedComments/threadedComment1.xml",
                comments.as_bytes(),
            ),
            ("xl/persons/person.xml", persons.as_bytes()),
        ]);
        let policy = HiddenContentPolicy::default().with_collaboration(Disposition::ScanAsContent);
        let (out, stripped) = sanitize(&package, policy, &SheetClassifier).unwrap();
        let out = out.unwrap();
        // No schema describes a threaded comment or its people, so neither
        // can be rebuilt, and both go.
        let names = part_names(&out);
        assert!(
            !names
                .iter()
                .any(|name| name.contains("threaded") || name.contains("person")),
            "{names:?}"
        );
        assert_eq!(count(&stripped, HiddenContent::ApplicationData), 2);
        crate::opc::check::check(&out, policy, &SheetClassifier).unwrap();
    }

    #[test]
    fn the_revision_log_is_stripped_even_when_collaboration_is_redacted() {
        let policy = HiddenContentPolicy::default().with_collaboration(Disposition::ScanAsContent);
        let (out, stripped) = sanitize(&workbook(), policy, &SheetClassifier).unwrap();
        let names = part_names(&out.unwrap());
        assert!(
            !names.iter().any(|name| name.contains("revisions")),
            "{names:?}"
        );
        assert_eq!(count(&stripped, HiddenContent::TrackedChange), 1);
    }

    #[test]
    fn comment_notes_go_with_stripped_comments() {
        let (out, stripped) = sanitize(
            &workbook(),
            HiddenContentPolicy::default(),
            &SheetClassifier,
        )
        .unwrap();
        let vml = read_text_part(&out.unwrap(), "xl/drawings/vmlDrawing1.vml");
        assert!(!vml.contains("Note") && vml.contains("Button"), "{vml}");
        assert_eq!(count(&stripped, HiddenContent::Comment), 1);

        // Redacted comments keep their notes.
        let policy = HiddenContentPolicy::default().with_collaboration(Disposition::ScanAsContent);
        let (out, _) = sanitize(&workbook(), policy, &SheetClassifier).unwrap();
        let vml = read_text_part(&out.unwrap(), "xl/drawings/vmlDrawing1.vml");
        assert!(vml.contains("Note"), "{vml}");
    }

    /// A crafted drawing nests one note in another; both go without a panic.
    #[test]
    fn nested_notes_are_cut_whole() {
        let raw = r#"<xml xmlns:v="urn:schemas-microsoft-com:vml" xmlns:x="urn:schemas-microsoft-com:office:excel"><v:shape>a<v:shape><x:ClientData ObjectType="Note"/></v:shape><x:ClientData ObjectType="Note"/></v:shape><v:rect/></xml>"#;
        let edited = expect_edit(raw, &CommentNotes);
        assert_eq!(
            edited.text,
            r#"<xml xmlns:v="urn:schemas-microsoft-com:vml" xmlns:x="urn:schemas-microsoft-com:office:excel"><v:rect/></xml>"#
        );
    }

    /// A note's kind is read as XML reads it, its references decoded.
    #[test]
    fn an_escaped_note_kind_is_a_note() {
        let raw = r#"<xml xmlns:v="urn:schemas-microsoft-com:vml" xmlns:x="urn:schemas-microsoft-com:office:excel"><v:shape><x:ClientData ObjectType="Not&#101;"/></v:shape></xml>"#;
        let edited = expect_edit(raw, &CommentNotes);
        assert!(!edited.text.contains("v:shape"), "{}", edited.text);
    }

    /// One note counts once, however its text is split.
    #[test]
    fn a_note_counts_once_whatever_its_text_events() {
        let raw = r#"<xml xmlns:v="urn:schemas-microsoft-com:vml" xmlns:x="urn:schemas-microsoft-com:office:excel"><v:shape><v:textbox><div>Jane &amp; Co</div></v:textbox><x:ClientData ObjectType="Note"/></v:shape></xml>"#;
        let edited = expect_edit(raw, &CommentNoteText);
        assert_eq!(edited.count, 1);
        assert!(edited.text.contains("<div></div>"), "{}", edited.text);
    }

    #[test]
    fn flagging_every_comment_empties_each_note_and_keeps_its_shape() {
        let policy = HiddenContentPolicy::default().with_collaboration(Disposition::FlagEverything);
        let (out, stripped) = sanitize(&workbook(), policy, &SheetClassifier).unwrap();
        let vml = read_text_part(&out.unwrap(), "xl/drawings/vmlDrawing1.vml");
        assert!(!vml.contains("<div>Note</div>"), "{vml}");
        assert!(vml.contains(r#"ObjectType="Note""#), "{vml}");
        assert!(vml.contains("<x:Row>0</x:Row>"), "{vml}");
        // A text box's legacy markup is outside the VML schema, so a button's
        // label goes in the rebuild.
        assert!(!vml.contains("<div>"), "{vml}");
        assert_eq!(count(&stripped, HiddenContent::Comment), 1);
    }
}
