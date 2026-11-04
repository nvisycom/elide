//! The PresentationML element edits: what hides inside the parts a presentation
//! keeps.
//!
//! A presentation tracks no revisions, and its comments and their authors are
//! parts of their own, removed whole with the references to them. What stays
//! hidden in the kept parts is an editor fingerprint, the creation id each
//! slide carries, and, when comments are redacted, when a comment was written
//! and its start and due dates. A legacy comment's date and a modern one's
//! start and due dates are optional, so they go; a modern comment's creation
//! date is required, so it is set to one neutral date.

use elide_core::redaction::{HiddenContent, HiddenContentPolicy};

use super::SlideFormat;
use crate::opc::sanitize::{Edits, Sanitizer};
use crate::opc::xml_edit::{Declared, Local};
use crate::opc::{
    Collaboration, NEUTRAL_DATE, NeutralizeAttributes, Ns, PartKind, StripAttributes, TextAttribute,
};

/// The PowerPoint 2010 extension namespace (slide creation ids).
const P14: &str = "http://schemas.microsoft.com/office/powerpoint/2010/main";

/// The creation id each slide carries.
const EDITING_SESSION_IDS: Declared = Declared {
    content: HiddenContent::EditingSessionId,
    remove: &[(P14, "creationId")],
    unwrap: &[],
    attributes: &[],
};

/// When a legacy comment was written, and a modern comment's start and due
/// dates.
const COMMENT_DATES: StripAttributes = StripAttributes {
    content: HiddenContent::Comment,
    attributes: &[
        TextAttribute::new((Ns::Any, Local::Exact("cm")), (Ns::None, "dt")),
        TextAttribute::new((Ns::Any, Local::Exact("cm")), (Ns::None, "startDate")),
        TextAttribute::new((Ns::Any, Local::Exact("cm")), (Ns::None, "dueDate")),
    ],
};

/// When a modern comment or reply was written, a date the format requires.
const CREATION_DATES: NeutralizeAttributes = NeutralizeAttributes {
    content: HiddenContent::Comment,
    attributes: &[
        TextAttribute::new((Ns::Any, Local::Exact("cm")), (Ns::None, "created")),
        TextAttribute::new((Ns::Any, Local::Exact("reply")), (Ns::None, "created")),
    ],
    value: NEUTRAL_DATE,
};

/// The text attributes a presentation surfaces for detection: its section and
/// custom-show names, a comment's title, and the names, initials, and accounts
/// of its comment authors.
pub(super) const TEXT_ATTRIBUTES: &[TextAttribute] = &[
    TextAttribute::new((Ns::Any, Local::Exact("cm")), (Ns::None, "title")).of(Collaboration::Text),
    TextAttribute::new((Ns::Any, Local::Exact("section")), (Ns::None, "name")),
    TextAttribute::new((Ns::Any, Local::Exact("custShow")), (Ns::None, "name")),
    TextAttribute::new((Ns::Any, Local::Exact("cmAuthor")), (Ns::None, "name"))
        .of(Collaboration::Author),
    TextAttribute::new((Ns::Any, Local::Exact("cmAuthor")), (Ns::None, "initials"))
        .of(Collaboration::Author),
    TextAttribute::new((Ns::Any, Local::Exact("author")), (Ns::None, "name"))
        .of(Collaboration::Author),
    TextAttribute::new((Ns::Any, Local::Exact("author")), (Ns::None, "initials"))
        .of(Collaboration::Author),
    TextAttribute::new((Ns::Any, Local::Exact("author")), (Ns::None, "userId"))
        .of(Collaboration::Author),
];

impl Sanitizer for SlideFormat {
    fn edits(&self, kind: PartKind, _policy: HiddenContentPolicy) -> Edits<'static> {
        match kind {
            PartKind::Main | PartKind::Story => {
                vec![&EDITING_SESSION_IDS]
            }
            PartKind::Comments => vec![&COMMENT_DATES, &CREATION_DATES],
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use elide_core::redaction::{Disposition, HiddenContentPolicy};

    use super::*;
    use crate::fixtures::{pack_parts, read_part};
    use crate::ooxml::OoxmlPackage;
    use crate::opc::sanitize::sanitize;

    const PML_NS: &str = "http://schemas.openxmlformats.org/presentationml/2006/main";

    #[test]
    fn a_modern_comment_keeps_a_neutral_creation_date() {
        const P188: &str = "http://schemas.microsoft.com/office/powerpoint/2018/8/main";
        const PC: &str = "http://schemas.microsoft.com/office/powerpoint/2013/main/command";
        let types = r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/><Override PartName="/ppt/comments/modernComment1.xml" ContentType="application/vnd.ms-powerpoint.comments+xml"/><Override PartName="/ppt/authors.xml" ContentType="application/vnd.ms-powerpoint.authors+xml"/></Types>"#;
        let presentation = format!(r#"<?xml version="1.0"?><p:presentation xmlns:p="{PML_NS}"/>"#);
        let comments = format!(
            r#"<?xml version="1.0"?><p188:cmLst xmlns:p188="{P188}" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:pc="{PC}"><p188:cm id="{{C}}" authorId="{{A}}" created="2024-01-02T03:04:05.123" startDate="2024-01-03T00:00:00" title="For Jane Roe"><pc:sldMkLst><pc:docMk/><pc:sldMk cId="1" sldId="256"/></pc:sldMkLst><p188:replyLst><p188:reply id="{{R}}" authorId="{{A}}" created="2024-01-04T00:00:00"><p188:txBody><a:bodyPr/><a:p><a:r><a:t>done</a:t></a:r></a:p></p188:txBody></p188:reply></p188:replyLst><p188:txBody><a:bodyPr/><a:p><a:r><a:t>check</a:t></a:r></a:p></p188:txBody></p188:cm></p188:cmLst>"#
        );
        let authors = format!(
            r#"<?xml version="1.0"?><p188:authorLst xmlns:p188="{P188}"><p188:author id="{{A}}" name="Jane Roe" initials="JR" userId="jane@example.com" providerId="AD"/></p188:authorLst>"#
        );
        let package = pack_parts(&[
            ("[Content_Types].xml", types.as_bytes()),
            ("ppt/presentation.xml", presentation.as_bytes()),
            ("ppt/comments/modernComment1.xml", comments.as_bytes()),
            ("ppt/authors.xml", authors.as_bytes()),
        ]);
        let policy = HiddenContentPolicy::default().with_collaboration(Disposition::ScanAsContent);
        let (out, _) = sanitize(&package, policy, &SlideFormat).unwrap();
        let out = out.unwrap();
        let comments =
            String::from_utf8(read_part(&out, "ppt/comments/modernComment1.xml").unwrap()).unwrap();
        assert_eq!(comments.matches(NEUTRAL_DATE).count(), 2, "{comments}");
        assert!(
            !comments.contains("2024") && !comments.contains("startDate"),
            "{comments}"
        );

        let texts: Vec<String> = OoxmlPackage::<SlideFormat>::open(&out)
            .unwrap()
            .extract()
            .blocks
            .iter()
            .map(|block| block.text.to_string())
            .collect();
        for text in ["For Jane Roe", "Jane Roe", "jane@example.com"] {
            assert!(texts.iter().any(|t| t == text), "{text} not in {texts:?}");
        }
        crate::opc::check::check(&out, policy, &SlideFormat).unwrap();
    }

    #[test]
    fn comment_authors_are_surfaced_and_dates_dropped() {
        let types = r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/><Override PartName="/ppt/commentAuthors.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.commentAuthors+xml"/><Override PartName="/ppt/comments/comment1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.comments+xml"/></Types>"#;
        let presentation = format!(r#"<?xml version="1.0"?><p:presentation xmlns:p="{PML_NS}"/>"#);
        let authors = format!(
            r#"<?xml version="1.0"?><p:cmAuthorLst xmlns:p="{PML_NS}"><p:cmAuthor id="0" name="Jane Roe" initials="JR" lastIdx="1" clrIdx="0"/></p:cmAuthorLst>"#
        );
        let comments = format!(
            r#"<?xml version="1.0"?><p:cmLst xmlns:p="{PML_NS}"><p:cm authorId="0" dt="2024-01-02T03:04:05.000" idx="1"><p:text>check</p:text></p:cm></p:cmLst>"#
        );
        let package = pack_parts(&[
            ("[Content_Types].xml", types.as_bytes()),
            ("ppt/presentation.xml", presentation.as_bytes()),
            ("ppt/commentAuthors.xml", authors.as_bytes()),
            ("ppt/comments/comment1.xml", comments.as_bytes()),
        ]);
        let policy = HiddenContentPolicy::default().with_collaboration(Disposition::ScanAsContent);
        let (out, _) = sanitize(&package, policy, &SlideFormat).unwrap();
        let out = out.unwrap();
        let comments =
            String::from_utf8(read_part(&out, "ppt/comments/comment1.xml").unwrap()).unwrap();
        assert!(!comments.contains("dt="), "{comments}");

        let texts: Vec<String> = OoxmlPackage::<SlideFormat>::open(&out)
            .unwrap()
            .extract()
            .blocks
            .iter()
            .map(|block| block.text.to_string())
            .collect();
        assert!(texts.iter().any(|text| text == "Jane Roe"), "{texts:?}");
        assert!(texts.iter().any(|text| text == "JR"), "{texts:?}");
    }
}
