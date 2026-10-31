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

use elide_core::redaction::{Disposition, HiddenContent, HiddenContentPolicy};

use super::WordFormat;
use crate::opc::PartKind;
use crate::opc::sanitize::{Edits, Sanitizer};
use crate::opc::xml_edit::{Declared, Local};

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
        ];
        if policy.collaboration == Disposition::Strip {
            edits.push(&REVISIONS);
            edits.push(&COMMENT_ANCHORS);
        }
        edits
    }
}
