//! The SpreadsheetML element edits: what hides inside the parts a workbook
//! keeps.
//!
//! - **Editor fingerprints**, always: the revision ids Excel stamps on the
//!   workbook, sheets, and tables, the absolute path the workbook was last
//!   saved to, and the file-sharing reservation (a user name and a password
//!   hash).
//! - **External references**, always: the workbook's list of linked external
//!   workbooks, whose parts are removed.
//! - **Comment notes**, under [`Disposition::Strip`]: the legacy VML shape
//!   that draws each comment's note, left behind by the comments part.
//! - **Threaded-comment dates and mentions**, when comments are redacted: when
//!   a comment was written, and its mentions, which point into the comment's
//!   text by offset and would point at the wrong text once it is redacted (the
//!   mentioned name stays in the text and is redacted with it).

use std::ops::Range;

use elide_core::redaction::{Disposition, HiddenContent, HiddenContentPolicy};
use elide_core::{Error, Result};
use quick_xml::NsReader;
use quick_xml::events::Event;

use super::SheetClassifier;
use crate::opc::sanitize::{Edits, Sanitizer};
use crate::opc::xml_edit::{Declared, Edit, Edited, Local, bound};
use crate::opc::{Ns, PartKind, StripAttributes, TextAttribute};

/// The SpreadsheetML main namespace.
const SML: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
/// The Excel 2010 application-compatibility namespace (the saved-to path).
const X15AC: &str = "http://schemas.microsoft.com/office/spreadsheetml/2010/11/ac";
/// The Excel revision-id namespaces.
const XR: &str = "http://schemas.microsoft.com/office/spreadsheetml/2014/revision";
const XR2: &str = "http://schemas.microsoft.com/office/spreadsheetml/2015/revision2";
const XR3: &str = "http://schemas.microsoft.com/office/spreadsheetml/2016/revision3";
const XR6: &str = "http://schemas.microsoft.com/office/spreadsheetml/2016/revision6";
const XR10: &str = "http://schemas.microsoft.com/office/spreadsheetml/2016/revision10";

/// The revision ids Excel stamps on the workbook, sheets, and tables.
const EDITING_SESSION_IDS: Declared = Declared {
    content: HiddenContent::EditingSessionId,
    remove: &[(XR, "revisionPtr")],
    unwrap: &[],
    attributes: &[
        (XR, Local::Any),
        (XR2, Local::Any),
        (XR3, Local::Any),
        (XR6, Local::Any),
        (XR10, Local::Any),
    ],
};

/// The absolute path the workbook was last saved to.
const FILE_PATH: Declared = Declared {
    content: HiddenContent::FilePath,
    remove: &[(X15AC, "absPath")],
    unwrap: &[],
    attributes: &[],
};

/// The file-sharing reservation: a user name and a password hash.
const CREDENTIALS: Declared = Declared {
    content: HiddenContent::Credential,
    remove: &[(SML, "fileSharing")],
    unwrap: &[],
    attributes: &[],
};

/// The workbook's list of linked external workbooks.
const EXTERNAL_WORKBOOKS: Declared = Declared {
    content: HiddenContent::Link,
    remove: &[(SML, "externalReferences")],
    unwrap: &[],
    attributes: &[],
};

/// The threaded-comments namespace.
const THREADED: &str = "http://schemas.microsoft.com/office/spreadsheetml/2018/threadedcomments";

/// When a threaded comment was written.
const COMMENT_DATES: StripAttributes = StripAttributes {
    content: HiddenContent::Comment,
    attributes: &[TextAttribute::new(
        (Ns::In(THREADED), Local::Exact("threadedComment")),
        (Ns::None, "dT"),
    )],
};

/// A threaded comment's mentions.
const MENTIONS: Declared = Declared {
    content: HiddenContent::Comment,
    remove: &[(THREADED, "mentions")],
    unwrap: &[],
    attributes: &[],
};

/// The legacy VML namespace.
const VML: &str = "urn:schemas-microsoft-com:vml";
/// The legacy Excel drawing namespace, whose `ClientData` says what a VML
/// shape is.
const EXCEL: &str = "urn:schemas-microsoft-com:office:excel";

/// The VML shapes that draw comment notes: a `v:shape` whose `x:ClientData`
/// has `ObjectType="Note"`. A shape's kind is in a child, so this edit reads
/// each shape whole before cutting it.
struct CommentNotes;

impl Edit for CommentNotes {
    fn content(&self) -> Option<HiddenContent> {
        Some(HiddenContent::Comment)
    }

    fn apply(&self, raw: &str) -> Result<Option<Edited>> {
        let mut reader = NsReader::from_str(raw);
        let bom = if raw.starts_with('\u{feff}') { 3 } else { 0 };
        let mut last = bom;
        // The open shapes: each one's start byte and whether it is a note.
        let mut shapes: Vec<(usize, bool)> = Vec::new();
        // Whether each open element is a shape.
        let mut open: Vec<bool> = Vec::new();
        let mut cuts: Vec<Range<usize>> = Vec::new();
        loop {
            let (resolved, event) = reader
                .read_resolved_event()
                .map_err(|e| Error::malformed(format!("malformed part XML: {e}")))?;
            let namespace = bound(&resolved);
            let span = last..reader.buffer_position() as usize + bom;
            last = span.end;
            match event {
                Event::Eof => break,
                Event::Start(ref e) | Event::Empty(ref e) => {
                    let local = e.local_name();
                    let local = local.into_inner();
                    let in_ns = |ns: &str| namespace.as_deref() == Some(ns);
                    if in_ns(EXCEL) && local == "ClientData" {
                        let note = e.attributes().flatten().any(|attr| {
                            attr.key.local_name().as_ref() == "ObjectType"
                                && attr.value.as_ref() == "Note"
                        });
                        if note && let Some(shape) = shapes.last_mut() {
                            shape.1 = true;
                        }
                    }
                    if matches!(event, Event::Start(_)) {
                        let shape = in_ns(VML) && local == "shape";
                        if shape {
                            shapes.push((span.start, false));
                        }
                        open.push(shape);
                    }
                }
                Event::End(_) => {
                    if open.pop() == Some(true)
                        && let Some((start, note)) = shapes.pop()
                        && note
                    {
                        cuts.push(start..span.end);
                    }
                }
                _ => {}
            }
        }
        if cuts.is_empty() {
            return Ok(None);
        }
        let count = cuts.len();
        let mut text = String::with_capacity(raw.len());
        let mut cursor = 0;
        for cut in cuts {
            // A note is never inside another, so the cuts are disjoint and in
            // order.
            text.push_str(&raw[cursor..cut.start]);
            cursor = cut.end;
        }
        text.push_str(&raw[cursor..]);
        Ok(Some(Edited { text, count }))
    }
}

/// A sheet's name, shown on its tab.
pub(super) const SHEET_NAME: TextAttribute =
    TextAttribute::new((Ns::Any, Local::Exact("sheet")), (Ns::None, "name"));

/// A defined name.
pub(super) const DEFINED_NAME: TextAttribute =
    TextAttribute::new((Ns::Any, Local::Exact("definedName")), (Ns::None, "name"));

/// A table's name.
pub(super) const TABLE_NAME: TextAttribute =
    TextAttribute::new((Ns::Any, Local::Exact("table")), (Ns::None, "name"));

/// A table's display name, the one formulas use.
pub(super) const TABLE_DISPLAY_NAME: TextAttribute =
    TextAttribute::new((Ns::Any, Local::Exact("table")), (Ns::None, "displayName"));

/// A table column's name, its header.
pub(super) const TABLE_COLUMN_NAME: TextAttribute =
    TextAttribute::new((Ns::Any, Local::Exact("tableColumn")), (Ns::None, "name"));

/// Where a hyperlink inside the workbook goes: a defined name or a
/// sheet-qualified cell.
pub(super) const HYPERLINK_LOCATION: TextAttribute =
    TextAttribute::new((Ns::Any, Local::Exact("hyperlink")), (Ns::None, "location"));

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
    TextAttribute::new((Ns::Any, Local::Exact("hyperlink")), (Ns::None, "display")),
    TextAttribute::new(
        (Ns::Any, Local::Exact("dataValidation")),
        (Ns::None, "promptTitle"),
    ),
    TextAttribute::new(
        (Ns::Any, Local::Exact("dataValidation")),
        (Ns::None, "prompt"),
    ),
    TextAttribute::new(
        (Ns::Any, Local::Exact("dataValidation")),
        (Ns::None, "errorTitle"),
    ),
    TextAttribute::new(
        (Ns::Any, Local::Exact("dataValidation")),
        (Ns::None, "error"),
    ),
    TextAttribute::new((Ns::Any, Local::Exact("scenario")), (Ns::None, "name")),
    TextAttribute::new((Ns::Any, Local::Exact("scenario")), (Ns::None, "comment")),
    TextAttribute::new((Ns::Any, Local::Exact("scenario")), (Ns::None, "user")),
    TextAttribute::new(
        (Ns::Any, Local::Exact("customWorkbookView")),
        (Ns::None, "name"),
    ),
    TextAttribute::new((Ns::Any, Local::Exact("person")), (Ns::None, "displayName")),
    TextAttribute::new((Ns::Any, Local::Exact("person")), (Ns::None, "userId")),
];

impl Sanitizer for SheetClassifier {
    fn edits(&self, kind: PartKind, policy: HiddenContentPolicy) -> Edits<'static> {
        match kind {
            PartKind::Comments => vec![&EDITING_SESSION_IDS, &COMMENT_DATES, &MENTIONS],
            PartKind::Drawing if policy.collaboration == Disposition::Strip => {
                vec![&EDITING_SESSION_IDS, &CommentNotes]
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
    use crate::fixtures::{pack_parts, part_names, read_part};
    use crate::opc::sanitize::sanitize;

    const SML_NS: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";

    /// A workbook with a revision log and a legacy drawing holding one comment
    /// note and one button.
    fn workbook() -> Vec<u8> {
        let types = r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="vml" ContentType="application/vnd.openxmlformats-officedocument.vmlDrawing"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/revisions/revisionHeaders.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.revisionHeaders+xml"/></Types>"#;
        let workbook = format!(r#"<?xml version="1.0"?><workbook xmlns="{SML_NS}"/>"#);
        let revisions = format!(
            r#"<?xml version="1.0"?><headers xmlns="{SML_NS}"><header userName="Jane Roe"/></headers>"#
        );
        let shape = |kind: &str| {
            format!(
                r##""<v:shape id="s{kind}" type="#_x0000_t202"><v:textbox><div>{kind}</div></v:textbox><x:ClientData ObjectType="{kind}"><x:Row>0</x:Row></x:ClientData></v:shape>""##
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
    fn redacted_threaded_comments_keep_their_text_and_authors_only() {
        let types = r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/threadedComments/threadedComment1.xml" ContentType="application/vnd.ms-excel.threadedcomments+xml"/><Override PartName="/xl/persons/person.xml" ContentType="application/vnd.ms-excel.person+xml"/></Types>"#;
        let workbook = format!(r#"<?xml version="1.0"?><workbook xmlns="{SML_NS}"/>"#);
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
        let policy = HiddenContentPolicy::default().with_collaboration(Disposition::Redact);
        let (out, _) = sanitize(&package, policy, &SheetClassifier).unwrap();
        let out = out.unwrap();
        let comments =
            String::from_utf8(read_part(&out, "xl/threadedComments/threadedComment1.xml").unwrap())
                .unwrap();
        assert!(
            !comments.contains("dT=") && !comments.contains("mention"),
            "{comments}"
        );

        let markup = crate::xlsx::Xlsx::open(&out).unwrap().markup();
        let texts: Vec<&str> = markup.blocks.iter().map(|b| b.text.as_str()).collect();
        for text in ["@Jane Roe please check", "Jane Roe", "jane@example.com"] {
            assert!(texts.contains(&text), "{text} not in {texts:?}");
        }
        crate::opc::check::check(&out, policy, &SheetClassifier).unwrap();
    }

    #[test]
    fn the_revision_log_is_stripped_even_when_collaboration_is_redacted() {
        let policy = HiddenContentPolicy::default().with_collaboration(Disposition::Redact);
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
        let vml =
            String::from_utf8(read_part(&out.unwrap(), "xl/drawings/vmlDrawing1.vml").unwrap())
                .unwrap();
        assert!(!vml.contains("Note") && vml.contains("Button"), "{vml}");
        assert_eq!(count(&stripped, HiddenContent::Comment), 1);

        // Redacted comments keep their notes.
        let policy = HiddenContentPolicy::default().with_collaboration(Disposition::Redact);
        let (out, _) = sanitize(&workbook(), policy, &SheetClassifier).unwrap();
        let vml =
            String::from_utf8(read_part(&out.unwrap(), "xl/drawings/vmlDrawing1.vml").unwrap())
                .unwrap();
        assert!(vml.contains("Note"), "{vml}");
    }
}
