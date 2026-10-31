//! The PresentationML element edits: what hides inside the parts a presentation
//! keeps.
//!
//! A presentation tracks no revisions, and its comments and their authors are
//! parts of their own, removed whole with the references to them. What stays
//! hidden in the kept parts is an editor fingerprint: the creation id each slide
//! carries.

use elide_core::redaction::HiddenContentPolicy;

use super::SlideFormat;
use crate::opc::PartKind;
use crate::opc::sanitize::{Edits, Sanitizer};
use crate::opc::xml_edit::Declared;

/// The PowerPoint 2010 extension namespace (slide creation ids).
const P14: &str = "http://schemas.microsoft.com/office/powerpoint/2010/main";

/// Editor fingerprints, removed always.
const FINGERPRINTS: Declared = Declared {
    remove: &[(P14, "creationId")],
    unwrap: &[],
    attributes: &[],
};

impl Sanitizer for SlideFormat {
    fn edits(&self, kind: PartKind, _policy: HiddenContentPolicy) -> Edits<'static> {
        match kind {
            PartKind::Main | PartKind::Story => {
                vec![("editor fingerprints", &FINGERPRINTS)]
            }
            _ => Vec::new(),
        }
    }
}
