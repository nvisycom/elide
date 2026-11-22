use elide_core::redaction::{Disposition, HiddenContent, HiddenContentPolicy};

use super::*;
use crate::fixtures::{pack_typed, part_names, read_text_part};
use crate::opc::sanitize::{Sanitized, sanitize};
use crate::opc::xml::edit::expect_edit;

/// A workbook with a revision log and a legacy drawing holding one comment
/// note and one button.
fn workbook() -> Vec<u8> {
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
    pack_typed(&[
        (
            "xl/workbook.xml",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml",
            workbook.as_bytes(),
        ),
        (
            "xl/revisions/revisionHeaders.xml",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.revisionHeaders+xml",
            revisions.as_bytes(),
        ),
        (
            "xl/drawings/vmlDrawing1.vml",
            "application/vnd.openxmlformats-officedocument.vmlDrawing",
            vml.as_bytes(),
        ),
    ])
}

fn count(stripped: &elide_codec::StrippedContent, content: HiddenContent) -> usize {
    stripped
        .iter()
        .filter(|entry| entry.content == content)
        .map(|entry| entry.count)
        .sum()
}

/// A threaded comment and its people, rebuilt from [MS-XLSX]'s schema: kept
/// and read when comments are scanned, its date and mentions dropped, and
/// removed when they are not.
#[test]
fn threaded_comments_are_read_when_scanned() {
    let workbook = format!(r#"<?xml version="1.0"?><workbook xmlns="{X}"/>"#);
    let comments = format!(
        r#"<?xml version="1.0"?><ThreadedComments xmlns="{THREADED}"><threadedComment ref="A1" dT="2024-01-02T03:04:05.00" personId="{{P}}" id="{{C}}"><text>@Jane Roe please check</text><mentions><mention mentionpersonId="{{P}}" mentionId="{{M}}" startIndex="0" length="9"/></mentions></threadedComment></ThreadedComments>"#
    );
    let persons = format!(
        r#"<?xml version="1.0"?><personList xmlns="{THREADED}"><person displayName="Jane Roe" id="{{P}}" userId="jane@example.com" providerId="PeoplePicker"/></personList>"#
    );
    let package = pack_typed(&[
        (
            "xl/workbook.xml",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml",
            workbook.as_bytes(),
        ),
        (
            "xl/threadedComments/threadedComment1.xml",
            "application/vnd.ms-excel.threadedcomments+xml",
            comments.as_bytes(),
        ),
        (
            "xl/persons/person.xml",
            "application/vnd.ms-excel.person+xml",
            persons.as_bytes(),
        ),
    ]);
    let policy = HiddenContentPolicy::default().with_collaboration(Disposition::ScanAsContent);
    let Sanitized { bytes: out, .. } = sanitize(&package, policy, &SheetFormat).unwrap();
    let out = out.unwrap();
    let comments = read_text_part(&out, "xl/threadedComments/threadedComment1.xml");
    assert!(
        comments.contains("<text>@Jane Roe please check</text>"),
        "{comments}"
    );
    assert!(!comments.contains("dT="), "{comments}");
    assert!(!comments.contains("mention"), "{comments}");
    let persons = read_text_part(&out, "xl/persons/person.xml");
    assert!(persons.contains(r#"displayName="Jane Roe""#), "{persons}");
    crate::opc::check::check(&out, policy, &SheetFormat).unwrap();

    let policy = HiddenContentPolicy::default();
    let Sanitized {
        bytes: out,
        stripped,
    } = sanitize(&package, policy, &SheetFormat).unwrap();
    let out = out.unwrap();
    assert_eq!(part_names(&out), ["[Content_Types].xml", "xl/workbook.xml"]);
    assert_eq!(count(&stripped, HiddenContent::ApplicationData), 0);
    crate::opc::check::check(&out, policy, &SheetFormat).unwrap();
}

#[test]
fn the_revision_log_is_stripped_even_when_collaboration_is_redacted() {
    let policy = HiddenContentPolicy::default().with_collaboration(Disposition::ScanAsContent);
    let Sanitized {
        bytes: out,
        stripped,
    } = sanitize(&workbook(), policy, &SheetFormat).unwrap();
    let names = part_names(&out.unwrap());
    assert!(
        !names.iter().any(|name| name.contains("revisions")),
        "{names:?}"
    );
    assert_eq!(count(&stripped, HiddenContent::TrackedChange), 1);
}

#[test]
fn comment_notes_go_with_stripped_comments() {
    let Sanitized {
        bytes: out,
        stripped,
    } = sanitize(&workbook(), HiddenContentPolicy::default(), &SheetFormat).unwrap();
    let vml = read_text_part(&out.unwrap(), "xl/drawings/vmlDrawing1.vml");
    assert!(!vml.contains("Note") && vml.contains("Button"), "{vml}");
    assert_eq!(count(&stripped, HiddenContent::Comment), 1);

    // Redacted comments keep their notes.
    let policy = HiddenContentPolicy::default().with_collaboration(Disposition::ScanAsContent);
    let Sanitized { bytes: out, .. } = sanitize(&workbook(), policy, &SheetFormat).unwrap();
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

/// Only the unqualified `ObjectType` is a shape's kind; a foreign one is
/// an undeclared attribute the rebuild drops.
#[test]
fn a_qualified_object_type_is_not_a_kind() {
    let raw = r#"<xml xmlns:v="urn:schemas-microsoft-com:vml" xmlns:x="urn:schemas-microsoft-com:office:excel" xmlns:z="urn:vendor"><v:shape><x:ClientData ObjectType="Button" z:ObjectType="Note"/></v:shape></xml>"#;
    assert!(CommentNotes.apply(raw).unwrap().is_none());
}

/// A note's kind that cannot be read refuses the drawing rather than
/// passing for no note.
#[test]
fn a_malformed_note_kind_is_refused() {
    let raw = r#"<xml xmlns:v="urn:schemas-microsoft-com:vml" xmlns:x="urn:schemas-microsoft-com:office:excel"><v:shape><x:ClientData Other="1" Other="2" ObjectType="Note"/></v:shape></xml>"#;
    assert!(CommentNotes.apply(raw).is_err());
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
    let Sanitized {
        bytes: out,
        stripped,
    } = sanitize(&workbook(), policy, &SheetFormat).unwrap();
    let vml = read_text_part(&out.unwrap(), "xl/drawings/vmlDrawing1.vml");
    assert!(!vml.contains("<div>Note</div>"), "{vml}");
    assert!(vml.contains(r#"ObjectType="Note""#), "{vml}");
    assert!(vml.contains("<x:Row>0</x:Row>"), "{vml}");
    // A text box's legacy markup is outside the VML schema, so a button's
    // label goes in the rebuild.
    assert!(!vml.contains("<div>"), "{vml}");
    assert_eq!(count(&stripped, HiddenContent::Comment), 1);
}
