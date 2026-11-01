//! The sanitizer's reads and edits of one dictionary or content-stream
//! operation.

use elide_core::redaction::HiddenContent;
use lopdf::content::Operation;
use lopdf::{Dictionary, Object};

use crate::lopdf_ext::DictionaryExt;

/// The entries of a marked-content property list that carry text of their own:
/// the replacement text, an alternate description, and an expansion.
const ALTERNATE_TEXT_KEYS: [&[u8]; 3] = [b"ActualText", b"Alt", b"E"];

/// The keys of an embedded file's parameters that time-stamp or fingerprint
/// it.
const FILE_METADATA_KEYS: [&[u8]; 4] = [b"CreationDate", b"ModDate", b"CheckSum", b"Mac"];

/// The sanitizer's reads and edits of one [`Dictionary`].
pub(super) trait SanitizeDictionaryExt {
    /// Remove every entry but the `kept` ones, returning each removed key with
    /// what `content` says it held (an entry it says nothing for goes
    /// unrecorded).
    fn retain_keys(
        &mut self,
        kept: &[&[u8]],
        content: impl Fn(&[u8]) -> Option<HiddenContent>,
    ) -> Vec<(Vec<u8>, HiddenContent)>;

    /// Whether this annotation is a link to a place in this same document: a
    /// `/Link` with a `/Dest`, or whose action is an internal `/GoTo` that
    /// chains no further action (a `/Next` can run script after the jump).
    fn is_internal_link(&self) -> bool;

    /// The hidden content this annotation is, by its subtype: a form widget, a
    /// link, rich media, or (every other subtype) a markup annotation.
    fn annotation_content(&self) -> HiddenContent;

    /// Whether this is an optional-content group or membership dictionary.
    fn is_content_group(&self) -> bool;

    /// Remove a file specification's description, or an embedded file's
    /// timestamps and checksum, returning how many were removed.
    fn strip_attachment_metadata(&mut self) -> usize;
}

impl SanitizeDictionaryExt for Dictionary {
    fn retain_keys(
        &mut self,
        kept: &[&[u8]],
        content: impl Fn(&[u8]) -> Option<HiddenContent>,
    ) -> Vec<(Vec<u8>, HiddenContent)> {
        let removed: Vec<Vec<u8>> = self
            .iter()
            .map(|(key, _)| key.clone())
            .filter(|key| !kept.contains(&key.as_slice()))
            .collect();
        removed
            .into_iter()
            .filter_map(|key| {
                self.remove(&key);
                let held = content(&key)?;
                Some((key, held))
            })
            .collect()
    }

    fn is_internal_link(&self) -> bool {
        if !self.has_subtype(b"Link") {
            return false;
        }
        match self.get(b"A").and_then(Object::as_dict) {
            Ok(action) => {
                let go_to = action
                    .get(b"S")
                    .and_then(Object::as_name)
                    .is_ok_and(|kind| kind == b"GoTo");
                go_to && action.get(b"Next").is_err()
            }
            Err(_) => self.get(b"Dest").is_ok(),
        }
    }

    fn annotation_content(&self) -> HiddenContent {
        let subtype = self.get(b"Subtype").and_then(Object::as_name);
        match subtype.unwrap_or_default() {
            b"Widget" => HiddenContent::FormField,
            b"Link" => HiddenContent::Link,
            b"Screen" | b"RichMedia" | b"Movie" | b"Sound" | b"3D" => HiddenContent::Control,
            _ => HiddenContent::Annotation,
        }
    }

    fn is_content_group(&self) -> bool {
        self.has_type(b"OCG") || self.has_type(b"OCMD")
    }

    fn strip_attachment_metadata(&mut self) -> usize {
        let mut count = 0;
        if self.has_type(b"Filespec") {
            count += self.remove_counted(b"Desc");
        }
        if self.has_type(b"EmbeddedFile")
            && let Ok(Object::Dictionary(params)) = self.get_mut(b"Params")
        {
            for key in FILE_METADATA_KEYS {
                count += params.remove_counted(key);
            }
        }
        count
    }
}

/// The sanitizer's edits of one content-stream [`Operation`].
pub(super) trait SanitizeOperationExt {
    /// Strip the alternate text from this `BDC` operation's inline property
    /// list, returning whether there was any.
    fn strip_alternate_text(&mut self) -> bool;
}

impl SanitizeOperationExt for Operation {
    fn strip_alternate_text(&mut self) -> bool {
        let Some(Object::Dictionary(properties)) = self.operands.get_mut(1) else {
            return false;
        };
        ALTERNATE_TEXT_KEYS
            .iter()
            .fold(false, |found, key| properties.remove(key).is_some() | found)
    }
}
