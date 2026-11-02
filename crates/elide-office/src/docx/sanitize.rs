//! The WordprocessingML element edits: what hides inside the parts a Word
//! document keeps.
//!
//! - **Editor fingerprints**, always: the `rsid` editing-session ids, paragraph
//!   and text ids, document ids, the session table, range permissions (which
//!   name their editors), and document protection (a password hash).
//! - **Tracked revisions**, under [`Disposition::Strip`]: insertions
//!   are accepted (their marks unwrapped, their content kept), deletions applied
//!   (removed with their content), moves resolved the same way, and every
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
//!   under [`Disposition::Redact`] too; the authors are surfaced instead (see
//!   [`TEXT_ATTRIBUTES`]).

use elide_core::redaction::{Disposition, HiddenContent, HiddenContentPolicy};

use super::WordFormat;
use crate::opc::names::{NameReference, NameRule, Numbering, ReferenceForm};
use crate::opc::sanitize::{Edits, Sanitizer};
use crate::opc::xml_edit::{Declared, Local};
use crate::opc::{Names, Ns, PartKind, StripAttributes, TextAttribute};

/// The WordprocessingML main namespace.
const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
/// The Word 2010 extension namespace (paragraph and text ids, document id).
const W14: &str = "http://schemas.microsoft.com/office/word/2010/wordml";
/// The Word 2012 extension namespace (document id).
const W15: &str = "http://schemas.microsoft.com/office/word/2012/wordml";

/// The editing-session ids: the `rsid` table and attributes, and the paragraph
/// and text ids.
const EDITING_SESSION_IDS: Declared = Declared {
    content: HiddenContent::EditingSessionId,
    remove: &[(W, "rsids")],
    unwrap: &[],
    attributes: &[
        (W, Local::Prefix("rsid")),
        (W14, Local::Exact("paraId")),
        (W14, Local::Exact("textId")),
    ],
};

/// The document ids.
const DOCUMENT_IDS: Declared = Declared {
    content: HiddenContent::DocumentId,
    remove: &[(W14, "docId"), (W15, "docId")],
    unwrap: &[],
    attributes: &[],
};

/// Range permissions, which name the users allowed to edit, and document and
/// write protection, which carry a password hash.
const CREDENTIALS: Declared = Declared {
    content: HiddenContent::Credential,
    remove: &[
        (W, "permStart"),
        (W, "permEnd"),
        (W, "documentProtection"),
        (W, "writeProtection"),
    ],
    unwrap: &[],
    attributes: &[],
};

/// Settings that reach outside the document: the attached template and an XSLT
/// save transform. Their relationships are external, so the reference cascade
/// would drop them too; naming them keeps a setting with no relationship from
/// surviving.
const LINKS: Declared = Declared {
    content: HiddenContent::Link,
    remove: &[(W, "attachedTemplate"), (W, "saveThroughXslt")],
    unwrap: &[],
    attributes: &[],
};

/// The mail-merge data source, inline or linked.
const DATA_CONNECTIONS: Declared = Declared {
    content: HiddenContent::DataConnection,
    remove: &[(W, "mailMerge")],
    unwrap: &[],
    attributes: &[],
};

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

/// The comment anchors in the stories.
const COMMENT_ANCHORS: Declared = Declared {
    content: HiddenContent::Comment,
    remove: &[
        (W, "commentRangeStart"),
        (W, "commentRangeEnd"),
        (W, "commentReference"),
    ],
    unwrap: &[],
    attributes: &[],
};

/// Document variables: name-value metadata no field surfaces.
const VARIABLES: Declared = Declared {
    content: HiddenContent::Properties,
    remove: &[(W, "docVars")],
    unwrap: &[],
    attributes: &[],
};

/// Content-control bindings into the custom XML data store.
const BINDINGS: Declared = Declared {
    content: HiddenContent::CustomData,
    remove: &[(W, "dataBinding")],
    unwrap: &[],
    attributes: &[],
};

/// A legacy form field's help and status text, and a content control's title
/// and tag: labels shown only while editing.
const FORM_LABELS: Declared = Declared {
    content: HiddenContent::FormField,
    remove: &[(W, "helpText"), (W, "statusText"), (W, "alias"), (W, "tag")],
    unwrap: &[],
    attributes: &[],
};

/// A table's caption and description.
const TABLE_ALTERNATIVE_TEXT: Declared = Declared {
    content: HiddenContent::AlternativeText,
    remove: &[(W, "tblCaption"), (W, "tblDescription")],
    unwrap: &[],
    attributes: &[],
};

/// When a comment was written.
const COMMENT_DATES: StripAttributes = StripAttributes {
    content: HiddenContent::Comment,
    attributes: &[
        TextAttribute::new((Ns::In(W), Local::Exact("comment")), (Ns::In(W), "date")),
        TextAttribute::new(
            (Ns::Any, Local::Exact("commentExtensible")),
            (Ns::Any, "dateUtc"),
        ),
    ],
};

/// When a tracked change was made.
const CHANGE_DATES: StripAttributes = StripAttributes {
    content: HiddenContent::TrackedChange,
    attributes: &[TextAttribute::new(
        (Ns::In(W), Local::Any),
        (Ns::In(W), "date"),
    )],
};

/// A bookmark's name, which a person may name after anyone.
const BOOKMARK_NAME: TextAttribute =
    TextAttribute::new((Ns::Any, Local::Exact("bookmarkStart")), (Ns::Any, "name"));

/// The bookmark a hyperlink inside the document goes to.
const HYPERLINK_ANCHOR: TextAttribute =
    TextAttribute::new((Ns::Any, Local::Exact("hyperlink")), (Ns::Any, "anchor"));

/// The longest bookmark name Word accepts, in characters.
const MAX_BOOKMARK_NAME: usize = 40;

/// A document's bookmarks: a redacted name is fitted to Word's rules for one
/// (see [`fit_bookmark_name`]) and unique, and a hyperlink to it follows. A
/// field naming it (`REF`, `PAGEREF`) does not.
pub(super) static NAMES: Names = Names {
    rules: &[NameRule {
        attribute: BOOKMARK_NAME,
        group: "bookmark",
        per_part: false,
        fit: fit_bookmark_name,
        max: Some(MAX_BOOKMARK_NAME),
        numbering: Numbering::Underscored,
    }],
    references: &[NameReference {
        attribute: HYPERLINK_ANCHOR,
        group: "bookmark",
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
    TextAttribute::new((Ns::Any, Local::Any), (Ns::Any, "author")),
    TextAttribute::new((Ns::Any, Local::Exact("comment")), (Ns::Any, "initials")),
    TextAttribute::new((Ns::Any, Local::Exact("presenceInfo")), (Ns::Any, "userId")),
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
        if policy.collaboration == Disposition::Strip {
            edits.push(&REVISIONS);
            edits.push(&COMMENT_ANCHORS);
        }
        edits
    }
}

#[cfg(test)]
mod tests {
    use elide_core::redaction::{Disposition, HiddenContent, HiddenContentPolicy};

    use super::*;
    use crate::fixtures::{pack_parts, read_part};
    use crate::ooxml::OoxmlPackage;
    use crate::opc::Replacement;
    use crate::opc::check::check;
    use crate::opc::sanitize::sanitize;

    const TYPES: &str = r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/comments.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.comments+xml"/></Types>"#;

    /// A Word document whose body carries a picture's description, a
    /// hyperlink's tooltip, a content control's title and tag, a table caption,
    /// and a tracked insertion, with a comments part beside it.
    fn document() -> Vec<u8> {
        let body = format!(
            r#"<?xml version="1.0"?><w:document xmlns:w="{W}" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"><w:body><w:p><w:hyperlink w:anchor="top" w:tooltip="Jane's page"><w:r><w:t>link</w:t></w:r></w:hyperlink><w:r><w:drawing><wp:inline><wp:docPr id="1" name="Picture 1" descr="C:\Users\jane\photo.jpg"/></wp:inline></w:drawing></w:r><w:ins w:id="1" w:author="Jane Roe" w:date="2024-01-02T03:04:05Z"><w:r><w:t>added</w:t></w:r></w:ins></w:p><w:sdt><w:sdtPr><w:alias w:val="Patient"/><w:tag w:val="patient-name"/></w:sdtPr><w:sdtContent><w:p/></w:sdtContent></w:sdt><w:tbl><w:tblPr><w:tblCaption w:val="Salaries of Jane Roe"/></w:tblPr><w:tr><w:tc><w:p/></w:tc></w:tr></w:tbl></w:body></w:document>"#
        );
        let comments = format!(
            r#"<?xml version="1.0"?><w:comments xmlns:w="{W}"><w:comment w:id="0" w:author="John Doe" w:initials="JD" w:date="2024-01-02T03:04:05Z"><w:p><w:r><w:t>check this</w:t></w:r></w:p></w:comment></w:comments>"#
        );
        pack_parts(&[
            ("[Content_Types].xml", TYPES.as_bytes()),
            ("_rels/.rels", br#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"/>"#),
            ("word/document.xml", body.as_bytes()),
            ("word/comments.xml", comments.as_bytes()),
        ])
    }

    fn part(package: &[u8], name: &str) -> String {
        String::from_utf8(read_part(package, name).unwrap()).unwrap()
    }

    #[test]
    fn a_redacted_bookmark_is_fitted_and_its_hyperlinks_follow() {
        let body = format!(
            r#"<?xml version="1.0"?><w:document xmlns:w="{W}"><w:body><w:p><w:bookmarkStart w:id="0" w:name="JaneRoe"/><w:r><w:t>x</w:t></w:r><w:bookmarkEnd w:id="0"/><w:hyperlink w:anchor="JaneRoe"><w:r><w:t>see</w:t></w:r></w:hyperlink></w:p></w:body></w:document>"#
        );
        let types = r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#;
        let package = pack_parts(&[
            ("[Content_Types].xml", types.as_bytes()),
            ("word/document.xml", body.as_bytes()),
        ]);
        let package = OoxmlPackage::<WordFormat>::open(&package).unwrap();
        let bookmark = body.find(r#"w:name="JaneRoe""#).unwrap() + r#"w:name=""#.len();
        let block = package
            .extract()
            .blocks
            .into_iter()
            .find(|block| block.start == bookmark)
            .expect("the bookmark name is surfaced");
        let out = package
            .rewrite_with_parts(
                &[Replacement {
                    part: block.part.clone(),
                    start: block.start,
                    end: block.end,
                    text: "[PERSON]".into(),
                }],
                &[],
            )
            .unwrap();
        let body = part(&out, "word/document.xml");
        assert!(body.contains(r#"w:name="PERSON_""#), "{body}");
        assert!(body.contains(r#"w:anchor="PERSON_""#), "{body}");
    }

    #[test]
    fn hidden_labels_and_alternative_text_are_stripped() {
        let (out, stripped) =
            sanitize(&document(), HiddenContentPolicy::default(), &WordFormat).unwrap();
        let out = out.unwrap();
        let body = part(&out, "word/document.xml");
        for hidden in [
            "photo.jpg",
            "Jane's page",
            "Patient",
            "patient-name",
            "Salaries",
        ] {
            assert!(!body.contains(hidden), "{hidden} in {body}");
        }
        let count = |content| {
            stripped
                .iter()
                .filter(|entry| entry.content == content)
                .map(|entry| entry.count)
                .sum::<usize>()
        };
        assert_eq!(count(HiddenContent::AlternativeText), 3);
        assert_eq!(count(HiddenContent::FormField), 2);
        check(&out, HiddenContentPolicy::default(), &WordFormat).unwrap();
    }

    #[test]
    fn redacted_collaboration_surfaces_its_authors_and_drops_its_dates() {
        let policy = HiddenContentPolicy::default().with_collaboration(Disposition::Redact);
        let (out, _) = sanitize(&document(), policy, &WordFormat).unwrap();
        let out = out.unwrap();
        for name in ["word/document.xml", "word/comments.xml"] {
            assert!(!part(&out, name).contains("w:date="), "a date in {name}");
        }
        let package = OoxmlPackage::<WordFormat>::open(&out).unwrap();
        let texts: Vec<String> = package
            .extract()
            .blocks
            .iter()
            .map(|block| block.text.to_string())
            .collect();
        for author in ["Jane Roe", "John Doe", "JD"] {
            assert!(
                texts.iter().any(|text| text == author),
                "{author} not in {texts:?}"
            );
        }
        check(&out, policy, &WordFormat).unwrap();
    }
}
