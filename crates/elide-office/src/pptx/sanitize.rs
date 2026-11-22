//! The PresentationML element edits: what hides inside the parts a presentation
//! keeps.
//!
//! A presentation tracks no revisions, and its comments and their authors are
//! parts of their own, removed whole with the references to them. What stays
//! hidden in the kept parts is an editor fingerprint, the creation id each
//! slide carries (which a modern comment's anchor repeats, with its shape's),
//! and, when comments are redacted, when a comment was written
//! and its start and due dates. A legacy comment's date and a modern one's
//! start and due dates are optional, so they go; a modern comment's creation
//! date is required, so it is set to one neutral date.

use elide_core::redaction::{HiddenContent, HiddenContentPolicy};

use super::SlideFormat;
use crate::opc::sanitize::{Edits, Sanitizer};
use crate::opc::xml::edit::{Declared, Local};
use crate::opc::xml::ns::{AC, P14, PC};
use crate::opc::{Collaboration, HiddenAttributes, NEUTRAL_DATE, Ns, PartKind, TextAttribute};

/// The creation id each slide carries.
const EDITING_SESSION_IDS: Declared =
    Declared::removing(HiddenContent::EditingSessionId, &[(P14, "creationId")]);

/// The creation ids a modern comment's anchor repeats: its slide's, on the
/// slide monikers, and its shape's, on the shape monikers.
const ANCHOR_CREATION_IDS: HiddenAttributes = HiddenAttributes::strip(
    HiddenContent::EditingSessionId,
    &[
        TextAttribute::new((Ns::In(PC), Local::Any), (Ns::Unqualified, "cId")),
        TextAttribute::new((Ns::In(AC), Local::Any), (Ns::Unqualified, "creationId")),
    ],
);

/// When a legacy comment was written, and a modern comment's start and due
/// dates.
const COMMENT_DATES: HiddenAttributes = HiddenAttributes::strip(
    HiddenContent::Comment,
    &[
        TextAttribute::unqualified("cm", "dt"),
        TextAttribute::unqualified("cm", "startDate"),
        TextAttribute::unqualified("cm", "dueDate"),
    ],
);

/// When a modern comment or reply was written, a date the format requires.
const CREATION_DATES: HiddenAttributes = HiddenAttributes::neutralize(
    HiddenContent::Comment,
    &[
        TextAttribute::unqualified("cm", "created"),
        TextAttribute::unqualified("reply", "created"),
    ],
    NEUTRAL_DATE,
);

/// The text attributes a presentation surfaces for detection: its section and
/// custom-show names, a comment's title, and the names, initials, and accounts
/// of its comment authors.
pub(super) const TEXT_ATTRIBUTES: &[TextAttribute] = &[
    TextAttribute::unqualified("cm", "title").of(Collaboration::Text),
    TextAttribute::unqualified("section", "name"),
    TextAttribute::unqualified("custShow", "name"),
    TextAttribute::unqualified("cmAuthor", "name").of(Collaboration::Author),
    TextAttribute::unqualified("cmAuthor", "initials").of(Collaboration::Author),
    TextAttribute::unqualified("author", "name").of(Collaboration::Author),
    TextAttribute::unqualified("author", "initials").of(Collaboration::Author),
    TextAttribute::unqualified("author", "userId").of(Collaboration::Author),
];

impl Sanitizer for SlideFormat {
    fn edits(&self, kind: PartKind, _policy: HiddenContentPolicy) -> Edits<'static> {
        match kind {
            PartKind::Main | PartKind::Story => {
                vec![&EDITING_SESSION_IDS]
            }
            PartKind::Comments => vec![&COMMENT_DATES, &CREATION_DATES, &ANCHOR_CREATION_IDS],
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use elide_core::redaction::{Disposition, HiddenContent, HiddenContentPolicy};

    use super::*;
    use crate::fixtures::{pack_typed, part_names, read_text_part};
    use crate::opc::sanitize::{Sanitized, sanitize};
    use crate::opc::xml::ns::{A, P, P188, R};
    use crate::text_package::OoxmlPackage;

    /// A modern comment and its authors, rebuilt from [MS-PPTX]'s schema:
    /// kept and read when comments are scanned, its dates dropped or made
    /// neutral, and removed when they are not.
    #[test]
    fn modern_comments_are_read_when_scanned() {
        let presentation = format!(r#"<?xml version="1.0"?><p:presentation xmlns:p="{P}"/>"#);
        let comments_ext = format!(
            r#"<p:ext uri="{{6950BFC3-D8DA-4A85-94F7-54DA5524770B}}"><p188:commentRel xmlns:p188="{P188}" r:id="rId1"/></p:ext>"#
        );
        let slide = format!(
            r#"<?xml version="1.0"?><p:sld xmlns:p="{P}" xmlns:r="{R}"><p:cSld/><p:extLst>{comments_ext}</p:extLst></p:sld>"#
        );
        let slide_rels = r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.microsoft.com/office/2018/10/relationships/comments" Target="../comments/modernComment1.xml"/></Relationships>"#;
        let comments = format!(
            r#"<?xml version="1.0"?><p188:cmLst xmlns:p188="{P188}" xmlns:a="{A}" xmlns:pc="{PC}" xmlns:ac="{AC}"><p188:cm id="{{C}}" authorId="{{A}}" created="2024-01-02T03:04:05.123" startDate="2024-01-03T00:00:00" title="For Jane Roe"><ac:deMkLst><pc:docMk/><pc:sldMk cId="1" sldId="256"/><ac:spMk id="2" creationId="{{S}}"/></ac:deMkLst><p188:replyLst><p188:reply id="{{R}}" authorId="{{A}}" created="2024-01-04T00:00:00"><p188:txBody><a:bodyPr/><a:p><a:r><a:t>done</a:t></a:r></a:p></p188:txBody></p188:reply></p188:replyLst><p188:txBody><a:bodyPr/><a:p><a:r><a:t>check</a:t></a:r></a:p></p188:txBody></p188:cm></p188:cmLst>"#
        );
        let authors = format!(
            r#"<?xml version="1.0"?><p188:authorLst xmlns:p188="{P188}"><p188:author id="{{A}}" name="Jane Roe" initials="JR" userId="jane@example.com" providerId="AD"/></p188:authorLst>"#
        );
        let package = pack_typed(&[
            (
                "ppt/presentation.xml",
                "application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml",
                presentation.as_bytes(),
            ),
            (
                "ppt/slides/slide1.xml",
                "application/vnd.openxmlformats-officedocument.presentationml.slide+xml",
                slide.as_bytes(),
            ),
            (
                "ppt/slides/_rels/slide1.xml.rels",
                "",
                slide_rels.as_bytes(),
            ),
            (
                "ppt/comments/modernComment1.xml",
                "application/vnd.ms-powerpoint.comments+xml",
                comments.as_bytes(),
            ),
            (
                "ppt/authors.xml",
                "application/vnd.ms-powerpoint.authors+xml",
                authors.as_bytes(),
            ),
        ]);
        let policy = HiddenContentPolicy::default().with_collaboration(Disposition::ScanAsContent);
        let Sanitized { bytes: out, .. } = sanitize(&package, policy, &SlideFormat).unwrap();
        let out = out.unwrap();
        let comments = read_text_part(&out, "ppt/comments/modernComment1.xml");
        // The anchor stays, but for the creation ids it repeats.
        assert!(
            comments.contains(
                r#"<ac:deMkLst><pc:docMk/><pc:sldMk sldId="256"/><ac:spMk id="2"/></ac:deMkLst>"#
            ),
            "{comments}"
        );
        // So does the slide's reference to its comments.
        let slide = read_text_part(&out, "ppt/slides/slide1.xml");
        assert!(slide.contains(&comments_ext), "{slide}");
        assert!(!comments.contains("startDate"), "{comments}");
        assert!(!comments.contains("2024-01-0"), "{comments}");
        crate::opc::check::check(&out, policy, &SlideFormat).unwrap();
        let texts = OoxmlPackage::<SlideFormat>::open(&out)
            .unwrap()
            .extract()
            .texts();
        for text in [
            "check",
            "done",
            "For Jane Roe",
            "Jane Roe",
            "JR",
            "jane@example.com",
        ] {
            assert!(texts.iter().any(|t| t == text), "{text}: {texts:?}");
        }

        let policy = HiddenContentPolicy::default();
        let Sanitized {
            bytes: out,
            stripped,
        } = sanitize(&package, policy, &SlideFormat).unwrap();
        let out = out.unwrap();
        let names = part_names(&out);
        assert_eq!(
            names,
            [
                "[Content_Types].xml",
                "ppt/presentation.xml",
                "ppt/slides/slide1.xml",
                "ppt/slides/_rels/slide1.xml.rels"
            ]
        );
        // The slide's reference goes with the comments, its extension with it.
        let slide = read_text_part(&out, "ppt/slides/slide1.xml");
        assert!(!slide.contains("6950BFC3"), "{slide}");
        assert!(
            stripped
                .iter()
                .all(|entry| entry.content != HiddenContent::ApplicationData)
        );
        crate::opc::check::check(&out, policy, &SlideFormat).unwrap();
    }

    #[test]
    fn comment_authors_are_surfaced_and_dates_dropped() {
        let presentation = format!(r#"<?xml version="1.0"?><p:presentation xmlns:p="{P}"/>"#);
        let authors = format!(
            r#"<?xml version="1.0"?><p:cmAuthorLst xmlns:p="{P}"><p:cmAuthor id="0" name="Jane Roe" initials="JR" lastIdx="1" clrIdx="0"/></p:cmAuthorLst>"#
        );
        let comments = format!(
            r#"<?xml version="1.0"?><p:cmLst xmlns:p="{P}"><p:cm authorId="0" dt="2024-01-02T03:04:05.000" idx="1"><p:text>check</p:text></p:cm></p:cmLst>"#
        );
        let package = pack_typed(&[
            (
                "ppt/presentation.xml",
                "application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml",
                presentation.as_bytes(),
            ),
            (
                "ppt/commentAuthors.xml",
                "application/vnd.openxmlformats-officedocument.presentationml.commentAuthors+xml",
                authors.as_bytes(),
            ),
            (
                "ppt/comments/comment1.xml",
                "application/vnd.openxmlformats-officedocument.presentationml.comments+xml",
                comments.as_bytes(),
            ),
        ]);
        let policy = HiddenContentPolicy::default().with_collaboration(Disposition::ScanAsContent);
        let Sanitized { bytes: out, .. } = sanitize(&package, policy, &SlideFormat).unwrap();
        let out = out.unwrap();
        let comments = read_text_part(&out, "ppt/comments/comment1.xml");
        assert!(!comments.contains("dt="), "{comments}");

        let texts = OoxmlPackage::<SlideFormat>::open(&out)
            .unwrap()
            .extract()
            .texts();
        assert!(texts.iter().any(|text| text == "Jane Roe"), "{texts:?}");
        assert!(texts.iter().any(|text| text == "JR"), "{texts:?}");
    }
}
