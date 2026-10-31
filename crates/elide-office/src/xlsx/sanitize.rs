//! The SpreadsheetML element edits: what hides inside the parts a workbook
//! keeps.
//!
//! - **Editor fingerprints**, always: the revision ids Excel stamps on the
//!   workbook, sheets, and tables, the absolute path the workbook was last
//!   saved to, and the file-sharing reservation (a user name and a password
//!   hash).
//! - **External references**, always: the workbook's list of linked external
//!   workbooks, whose parts are removed.

use elide_core::redaction::HiddenContentPolicy;

use super::SheetClassifier;
use crate::opc::PartKind;
use crate::opc::sanitize::{Edits, Sanitizer};
use crate::opc::xml_edit::{Declared, Local};

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

/// Editor fingerprints and external references, removed always.
const FINGERPRINTS: Declared = Declared {
    remove: &[
        (XR, "revisionPtr"),
        (X15AC, "absPath"),
        (SML, "fileSharing"),
    ],
    unwrap: &[],
    attributes: &[
        (XR, Local::Any),
        (XR2, Local::Any),
        (XR3, Local::Any),
        (XR6, Local::Any),
        (XR10, Local::Any),
    ],
};

/// The workbook's list of linked external workbooks.
const EXTERNAL: Declared = Declared {
    remove: &[(SML, "externalReferences")],
    unwrap: &[],
    attributes: &[],
};

impl Sanitizer for SheetClassifier {
    fn edits(&self, kind: PartKind, _policy: HiddenContentPolicy) -> Edits<'static> {
        match kind {
            PartKind::Main => vec![
                ("editor fingerprints", &FINGERPRINTS),
                ("external references", &EXTERNAL),
            ],
            PartKind::Story | PartKind::Structure | PartKind::Drawing | PartKind::Comments => {
                vec![("editor fingerprints", &FINGERPRINTS)]
            }
            _ => Vec::new(),
        }
    }
}
