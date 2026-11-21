//! The WordprocessingML element edits: what hides inside the parts a Word
//! document keeps.
//!
//! - **Editor fingerprints**, always: the `rsid` editing-session ids, paragraph
//!   and text ids, document ids, the session table, range permissions (which
//!   name their editors), and document protection (a password hash).
//! - **Tracked revisions**, under [`KeepOnlyEssential`](elide_core::redaction::Disposition::KeepOnlyEssential): insertions
//!   are accepted (their marks unwrapped, their content kept), deletions applied
//!   (removed with their content, a deleted table row or cell whole), moves resolved the same way, and every
//!   property-change record dropped, so the document is its final visible text.
//!   The setting that turns tracking back on goes too.
//! - **Comment anchors**, under the same action: the range marks and reference
//!   runs that tie text to a removed comment.
//! - **Settings that leave the document**, always: the attached template,
//!   mail-merge data sources, and XSLT save transforms.
//! - **Document variables**, always: metadata never surfaced to detection.
//! - **Custom XML bindings**, always: the content-control bindings into the
//!   custom XML data store, which is stripped.
//! - **Form-field and content-control labels**, always: a legacy form field's
//!   help and status text, and a content control's title and tag.
//! - **Table alternative text**, always: a table's caption and description.
//! - **Collaboration dates**, always: when a comment was written or a change
//!   made. A date cannot be redacted without breaking the document, so it goes
//!   under [`ScanAsContent`](elide_core::redaction::Disposition::ScanAsContent) too; the authors are surfaced instead (see
//!   [`TEXT_ATTRIBUTES`]).

use std::ops::Range;

use elide_core::Result;
use elide_core::redaction::{HiddenContent, HiddenContentPolicy};
use quick_xml::events::Event;

use super::WordFormat;
use crate::opc::names::{NameGroup, NameReference, NameRule, Numbering, ReferenceForm, Unique};
use crate::opc::sanitize::{Edits, Sanitizer};
use crate::opc::xml::edit::{Cut, Declared, Edit, Edited, Local, SpannedReader};
use crate::opc::xml::ns::{W, W14, W15, transitional};
use crate::opc::{
    Collaboration, HiddenAttributes, Names, Ns, PartKind, TextAttribute, malformed_xml,
};

/// The editing-session ids: the `rsid` table and attributes, and the paragraph
/// and text ids.
const EDITING_SESSION_IDS: Declared =
    Declared::removing(HiddenContent::EditingSessionId, &[(W, "rsids")]).with_attributes(&[
        (W, Local::Prefix("rsid")),
        (W14, Local::Exact("paraId")),
        (W14, Local::Exact("textId")),
    ]);

/// The document ids.
const DOCUMENT_IDS: Declared =
    Declared::removing(HiddenContent::DocumentId, &[(W14, "docId"), (W15, "docId")]);

/// Range permissions, which name the users allowed to edit, and document and
/// write protection, which carry a password hash.
const CREDENTIALS: Declared = Declared::removing(
    HiddenContent::Credential,
    &[
        (W, "permStart"),
        (W, "permEnd"),
        (W, "documentProtection"),
        (W, "writeProtection"),
    ],
);

/// Settings that reach outside the document: the attached template and an XSLT
/// save transform. Their relationships are external, so the reference cascade
/// would drop them too; naming them keeps a setting with no relationship from
/// surviving.
const LINKS: Declared = Declared::removing(
    HiddenContent::Link,
    &[(W, "attachedTemplate"), (W, "saveThroughXslt")],
);

/// The mail-merge data source, inline or linked.
const DATA_CONNECTIONS: Declared =
    Declared::removing(HiddenContent::DataConnection, &[(W, "mailMerge")]);

/// The elements that accept every tracked revision.
const REVISIONS: Declared = Declared {
    content: HiddenContent::TrackedChange,
    remove: &[
        (W, "del"),
        (W, "moveFrom"),
        (W, "moveFromRangeStart"),
        (W, "moveFromRangeEnd"),
        (W, "moveToRangeStart"),
        (W, "moveToRangeEnd"),
        (W, "rPrChange"),
        (W, "pPrChange"),
        (W, "sectPrChange"),
        (W, "tblPrChange"),
        (W, "tblPrExChange"),
        (W, "trPrChange"),
        (W, "tcPrChange"),
        (W, "tblGridChange"),
        (W, "numberingChange"),
        (W, "cellIns"),
        (W, "cellDel"),
        (W, "cellMerge"),
        (W, "customXmlInsRangeStart"),
        (W, "customXmlInsRangeEnd"),
        (W, "customXmlDelRangeStart"),
        (W, "customXmlDelRangeEnd"),
        (W, "customXmlMoveFromRangeStart"),
        (W, "customXmlMoveFromRangeEnd"),
        (W, "customXmlMoveToRangeStart"),
        (W, "customXmlMoveToRangeEnd"),
        (W, "trackRevisions"),
    ],
    unwrap: &[(W, "ins"), (W, "moveTo")],
    attributes: &[],
};

/// The table rows and cells a tracked change deletes: a `w:tr` whose
/// `w:trPr` holds `w:del`, and a `w:tc` whose `w:tcPr` holds `w:cellDel`.
/// The deletion is marked in a child, so this edit reads each row and cell
/// whole before cutting it; it runs before [`REVISIONS`], which removes the
/// markers.
struct DeletedTableParts;

impl Edit for DeletedTableParts {
    fn content(&self) -> Option<HiddenContent> {
        Some(HiddenContent::TrackedChange)
    }

    fn apply(&self, raw: &str) -> Result<Option<Edited>> {
        cut_deleted_table_parts(raw)
    }
}

/// An open element of a story as [`cut_deleted_table_parts`] reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Opened {
    /// A row or a cell, from this offset, and whether its deletion is marked.
    Table {
        start: usize,
        deleted: bool,
    },
    /// A row's or a cell's properties.
    Properties {
        marker: &'static str,
    },
    Other,
}

/// `raw` without the table rows and cells a tracked change deletes. The
/// count is of the rows and cells cut.
fn cut_deleted_table_parts(raw: &str) -> Result<Option<Edited>> {
    if !raw.contains("cellDel") && !raw.contains("trPr") {
        return Ok(None);
    }
    let mut reader = SpannedReader::new_ns(raw);
    let mut open: Vec<Opened> = Vec::new();
    let mut cuts: Vec<Range<usize>> = Vec::new();
    loop {
        let (namespace, event, span) = reader.read_resolved_event().map_err(malformed_xml)?;
        match event {
            Event::Eof => break,
            Event::Start(ref e) | Event::Empty(ref e) => {
                let local = e.local_name();
                let local = local.into_inner();
                let in_w = namespace.as_deref().map(transitional) == Some(W);
                // A marker in the properties of the open row or cell.
                if let Some(&Opened::Properties { marker }) = open.last()
                    && in_w
                    && local == marker
                    && let Some(Opened::Table { deleted, .. }) = open.iter_mut().rev().nth(1)
                {
                    *deleted = true;
                }
                if matches!(event, Event::Start(_)) {
                    let parent_table = matches!(open.last(), Some(Opened::Table { .. }));
                    open.push(match local {
                        "tr" | "tc" if in_w => Opened::Table {
                            start: span.start,
                            deleted: false,
                        },
                        "trPr" if in_w && parent_table => Opened::Properties { marker: "del" },
                        "tcPr" if in_w && parent_table => Opened::Properties { marker: "cellDel" },
                        _ => Opened::Other,
                    });
                }
            }
            Event::End(_) => {
                if let Some(Opened::Table {
                    start,
                    deleted: true,
                }) = open.pop()
                {
                    cuts.push(start..span.end);
                }
            }
            _ => {}
        }
    }
    let count = cuts.len();
    // The cuts may nest: a deleted row can hold a table with one of its own.
    let cuts: Vec<Cut<&str>> = cuts.into_iter().map(Cut::remove).collect();
    Ok(Edited::from_cuts(raw, cuts, count))
}

/// The comment anchors in the stories.
const COMMENT_ANCHORS: Declared = Declared::removing(
    HiddenContent::Comment,
    &[
        (W, "commentRangeStart"),
        (W, "commentRangeEnd"),
        (W, "commentReference"),
    ],
);

/// Document variables: name-value metadata no field surfaces.
const VARIABLES: Declared = Declared::removing(HiddenContent::Properties, &[(W, "docVars")]);

/// Content-control bindings into the custom XML data store.
const BINDINGS: Declared = Declared::removing(HiddenContent::CustomData, &[(W, "dataBinding")]);

/// A legacy form field's help and status text, and a content control's title
/// and tag: labels shown only while editing.
const FORM_LABELS: Declared = Declared::removing(
    HiddenContent::FormField,
    &[(W, "helpText"), (W, "statusText"), (W, "alias"), (W, "tag")],
);

/// A table's caption and description.
const TABLE_ALTERNATIVE_TEXT: Declared = Declared::removing(
    HiddenContent::AlternativeText,
    &[(W, "tblCaption"), (W, "tblDescription")],
);

/// When a comment was written.
const COMMENT_DATES: HiddenAttributes = HiddenAttributes::strip(
    HiddenContent::Comment,
    &[
        TextAttribute::new((Ns::In(W), Local::Exact("comment")), (Ns::In(W), "date")),
        TextAttribute::new(
            (Ns::Any, Local::Exact("commentExtensible")),
            (Ns::Any, "dateUtc"),
        ),
    ],
);

/// When a tracked change was made.
const CHANGE_DATES: HiddenAttributes = HiddenAttributes::strip(
    HiddenContent::TrackedChange,
    &[TextAttribute::new(
        (Ns::In(W), Local::Any),
        (Ns::In(W), "date"),
    )],
);

/// A bookmark's name, which a person may name after anyone.
const BOOKMARK_NAME: TextAttribute =
    TextAttribute::new((Ns::Any, Local::Exact("bookmarkStart")), (Ns::Any, "name"));

/// The bookmark a hyperlink inside the document goes to.
const HYPERLINK_ANCHOR: TextAttribute =
    TextAttribute::new((Ns::Any, Local::Exact("hyperlink")), (Ns::Any, "anchor"));

/// The bookmark names.
const BOOKMARKS: NameGroup = NameGroup::new("bookmark");

/// The longest bookmark name Word accepts, in characters.
const MAX_BOOKMARK_NAME: usize = 40;

/// A document's bookmarks: a redacted name is fitted to Word's rules for one
/// (see [`fit_bookmark_name`]) and unique, and a hyperlink to it follows. A
/// field naming it (`REF`, `PAGEREF`) does not.
pub(super) static NAMES: Names = Names {
    rules: &[NameRule {
        attribute: BOOKMARK_NAME,
        group: BOOKMARKS,
        unique: Unique::InPackage,
        fit: fit_bookmark_name,
        max: Some(MAX_BOOKMARK_NAME),
        numbering: Numbering::Underscored,
    }],
    references: &[NameReference {
        attribute: HYPERLINK_ANCHOR,
        group: BOOKMARKS,
        form: ReferenceForm::Exact,
    }],
};

/// `name` as a bookmark name: every character but a letter, a digit, and `_`
/// becomes `_`, leading characters that are not letters are dropped, and the
/// result is capped at [`MAX_BOOKMARK_NAME`] characters; an empty result is
/// `Bookmark`.
fn fit_bookmark_name(name: &str) -> String {
    let fitted: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .skip_while(|c| !c.is_alphabetic())
        .take(MAX_BOOKMARK_NAME)
        .collect();
    if fitted.is_empty() {
        "Bookmark".to_owned()
    } else {
        fitted
    }
}

/// The text attributes a Word document surfaces for detection: who wrote a
/// comment or made a change and the account they used, a bookmark's name and
/// the hyperlinks to it, a dropdown's entries, a text field's default, and a
/// simple field's instruction.
pub(super) const TEXT_ATTRIBUTES: &[TextAttribute] = &[
    BOOKMARK_NAME,
    HYPERLINK_ANCHOR,
    TextAttribute::new((Ns::Any, Local::Any), (Ns::Any, "author")).of(Collaboration::Author),
    TextAttribute::new((Ns::Any, Local::Exact("comment")), (Ns::Any, "initials"))
        .of(Collaboration::Author),
    TextAttribute::new((Ns::Any, Local::Exact("presenceInfo")), (Ns::Any, "userId"))
        .of(Collaboration::Author),
    TextAttribute::new((Ns::Any, Local::Exact("listEntry")), (Ns::Any, "val")),
    TextAttribute::new(
        (Ns::Any, Local::Exact("listItem")),
        (Ns::Any, "displayText"),
    ),
    TextAttribute::new((Ns::Any, Local::Exact("listItem")), (Ns::Any, "value")),
    TextAttribute::new((Ns::Any, Local::Exact("default")), (Ns::Any, "val")),
    TextAttribute::new((Ns::Any, Local::Exact("fldSimple")), (Ns::Any, "instr")),
];

impl Sanitizer for WordFormat {
    fn edits(&self, kind: PartKind, policy: HiddenContentPolicy) -> Edits<'static> {
        let wordprocessing = matches!(
            kind,
            PartKind::Main | PartKind::Story | PartKind::Comments | PartKind::Structure
        );
        if !wordprocessing {
            return Vec::new();
        }
        let mut edits: Edits<'static> = vec![
            &EDITING_SESSION_IDS,
            &DOCUMENT_IDS,
            &CREDENTIALS,
            &LINKS,
            &DATA_CONNECTIONS,
            &VARIABLES,
            &BINDINGS,
            &FORM_LABELS,
            &TABLE_ALTERNATIVE_TEXT,
            &COMMENT_DATES,
            &CHANGE_DATES,
        ];
        if !policy.collaboration.treats_as_content() {
            edits.push(&DeletedTableParts);
            edits.push(&REVISIONS);
            edits.push(&COMMENT_ANCHORS);
        }
        edits
    }
}

#[cfg(test)]
mod tests;
