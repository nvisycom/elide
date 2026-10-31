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

use elide_core::redaction::{Disposition, HiddenContentPolicy};

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

/// Editor fingerprints and settings that leave the document, removed always.
const FINGERPRINTS: Declared = Declared {
    remove: &[
        (W, "permStart"),
        (W, "permEnd"),
        (W, "rsids"),
        (W, "documentProtection"),
        (W, "writeProtection"),
        (W14, "docId"),
        (W15, "docId"),
    ],
    unwrap: &[],
    attributes: &[
        (W, Local::Prefix("rsid")),
        (W14, Local::Exact("paraId")),
        (W14, Local::Exact("textId")),
    ],
};

/// Settings that reach outside the document, removed always. Their
/// relationships are external, so the reference cascade would drop them too;
/// naming them keeps a setting with no relationship (an inline data source)
/// from surviving.
const EXTERNAL: Declared = Declared {
    remove: &[
        (W, "attachedTemplate"),
        (W, "mailMerge"),
        (W, "saveThroughXslt"),
    ],
    unwrap: &[],
    attributes: &[],
};

/// The elements that accept every tracked revision.
const REVISIONS: Declared = Declared {
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
    remove: &[(W, "docVars")],
    unwrap: &[],
    attributes: &[],
};

/// Content-control bindings into the custom XML data store.
const BINDINGS: Declared = Declared {
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
            ("editor fingerprints", &FINGERPRINTS),
            ("external references", &EXTERNAL),
            ("metadata", &VARIABLES),
            ("custom XML", &BINDINGS),
        ];
        if policy.collaboration == Disposition::Strip {
            edits.push(("revisions", &REVISIONS));
            edits.push(("comments", &COMMENT_ANCHORS));
        }
        edits
    }
}
