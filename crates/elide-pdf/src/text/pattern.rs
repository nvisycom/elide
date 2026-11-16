//! The tiling-pattern check: text a pattern draws is tiled across whatever the
//! pattern fills, which the walk does not follow, so a document with any is
//! refused.

use elide_core::{Error, Result};
use lopdf::content::Content;
use lopdf::{Dictionary, Document, Object, Stream};

use super::scope::{Scope, dict_or_ref, scope_xobject_id};
use super::walker::{MAX_XOBJECT_DEPTH, MAX_XOBJECT_WALKS};
use crate::document::object::{DictionaryExt, StreamExt};

/// Refuse a document whose tiling pattern draws text, itself or through a form
/// it draws: a pattern's content is tiled across whatever it fills, which the
/// walk does not follow, so its text could be neither detected nor deleted.
///
/// # Errors
///
/// [`ErrorKind::Redaction`](elide_core::ErrorKind::Redaction) for a pattern
/// that draws text, or [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput)
/// for content that cannot be read.
pub(super) fn refuse_text(doc: &Document) -> Result<()> {
    let mut walks = MAX_XOBJECT_WALKS;
    for (&id, object) in &doc.objects {
        let Object::Stream(pattern) = object else {
            continue;
        };
        if pattern
            .dict
            .get(b"PatternType")
            .and_then(Object::as_i64)
            .ok()
            != Some(1)
        {
            continue;
        }
        let resources = pattern
            .dict
            .get(b"Resources")
            .ok()
            .and_then(|r| dict_or_ref(doc, r));
        if draws_text(doc, pattern, resources, MAX_XOBJECT_DEPTH, &mut walks)? {
            return Err(Error::redaction(format!(
                "tiling pattern {id:?} draws text, which cannot be located for redaction"
            )));
        }
    }
    Ok(())
}

/// Whether the content of `stream` shows text, itself or through a Form
/// XObject it draws, with names resolved in `resources`.
fn draws_text(
    doc: &Document,
    stream: &Stream,
    resources: Option<&Dictionary>,
    depth: u8,
    walks: &mut u32,
) -> Result<bool> {
    *walks = walks.checked_sub(1).ok_or_else(|| {
        Error::resource_limit(format!(
            "patterns draw more than {MAX_XOBJECT_WALKS} Form XObjects"
        ))
    })?;
    let content = Content::decode(&stream.decoded()?)
        .map_err(|e| Error::malformed(format!("pattern content: {e}")))?;
    for op in &content.operations {
        match op.operator.as_str() {
            "Tj" | "TJ" | "'" | "\"" => return Ok(true),
            "Do" => {
                let Some(Object::Name(name)) = op.operands.first() else {
                    continue;
                };
                let scope = Scope {
                    inline: resources,
                    referenced: &[],
                };
                let Some(Ok(Object::Stream(form))) =
                    scope_xobject_id(doc, &scope, name).map(|id| doc.get_object(id))
                else {
                    continue;
                };
                if !form.dict.has_subtype(b"Form") {
                    continue;
                }
                let Some(depth) = depth.checked_sub(1) else {
                    return Err(Error::resource_limit(format!(
                        "a pattern nests Form XObjects deeper than {MAX_XOBJECT_DEPTH} levels"
                    )));
                };
                let inner = form
                    .dict
                    .get(b"Resources")
                    .ok()
                    .and_then(|r| dict_or_ref(doc, r))
                    .or(resources);
                if draws_text(doc, form, inner, depth, walks)? {
                    return Ok(true);
                }
            }
            _ => {}
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use elide_core::ErrorKind;
    use lopdf::content::Operation;
    use lopdf::{ObjectId, dictionary};

    use super::*;
    use crate::document::sanitize::SanitizeDocumentExt;

    /// A one-page document whose page paints a tiling pattern that draws a
    /// form showing text, each classifier stored as a reference when
    /// `indirect` names it, sanitized as at decode.
    fn painting_a_pattern(indirect: &[&str]) -> Document {
        let mut doc = Document::with_version("1.5");
        let mut value = |key: &str, direct: Object| -> Object {
            if indirect.contains(&key) {
                Object::Reference(doc.add_object(direct))
            } else {
                direct
            }
        };
        let subtype = value("Subtype", Object::Name(b"Form".to_vec()));
        let pattern_type = value("PatternType", Object::Integer(1));
        let font = doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        });
        let shows = Content {
            operations: vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec!["F1".into(), 12.into()]),
                Operation::new("Tj", vec![Object::string_literal("Jane Roe")]),
                Operation::new("ET", vec![]),
            ],
        };
        let form = doc.add_object(Stream::new(
            dictionary! {
                "Type" => "XObject", "Subtype" => subtype,
                "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
            },
            shows.encode().unwrap(),
        ));
        let pattern = doc.add_object(Stream::new(
            dictionary! {
                "Type" => "Pattern", "PatternType" => pattern_type,
                "Resources" => dictionary! { "XObject" => dictionary! { "T" => form } },
            },
            b"/T Do".to_vec(),
        ));
        let pages: ObjectId = doc.new_object_id();
        let content = doc.add_object(Stream::new(
            dictionary! {},
            b"/Pattern cs /P0 scn 0 0 10 10 re f".to_vec(),
        ));
        let page = doc.add_object(dictionary! {
            "Type" => "Page", "Parent" => pages, "Contents" => content,
            "Resources" => dictionary! { "Pattern" => dictionary! { "P0" => pattern } },
        });
        doc.objects.insert(
            pages,
            Object::Dictionary(dictionary! {
                "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1,
            }),
        );
        let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages });
        doc.trailer.set("Root", catalog);
        doc.sanitize().unwrap();
        doc
    }

    /// A pattern classified through a reference is still inspected.
    #[test]
    fn a_referenced_pattern_type_is_inspected() {
        let err = refuse_text(&painting_a_pattern(&["PatternType"])).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Redaction);
    }

    /// A form classified through a reference is still followed.
    #[test]
    fn a_referenced_form_subtype_is_followed() {
        let err = refuse_text(&painting_a_pattern(&["Subtype"])).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Redaction);
    }

    /// A tiling pattern's text is tiled across a fill the walk does not
    /// follow, so the document is refused, even through a form it draws.
    #[test]
    fn text_in_a_tiling_pattern_is_an_error() {
        let mut doc = Document::with_version("1.5");
        let font = doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        });
        let shows = Content {
            operations: vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec!["F1".into(), 12.into()]),
                Operation::new("Tj", vec![Object::string_literal("Jane Roe")]),
                Operation::new("ET", vec![]),
            ],
        };
        let text = doc.add_object(Stream::new(
            dictionary! {
                "Type" => "XObject", "Subtype" => "Form",
                "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
            },
            shows.encode().unwrap(),
        ));
        doc.add_object(Stream::new(
            dictionary! {
                "Type" => "Pattern", "PatternType" => 1,
                "Resources" => dictionary! { "XObject" => dictionary! { "T" => text } },
            },
            Content {
                operations: vec![Operation::new("Do", vec!["T".into()])],
            }
            .encode()
            .unwrap(),
        ));
        let err = refuse_text(&doc).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Redaction);
    }

    #[test]
    fn a_pattern_without_text_is_allowed() {
        let mut doc = Document::with_version("1.5");
        doc.add_object(Stream::new(
            dictionary! { "Type" => "Pattern", "PatternType" => 1 },
            b"0 0 10 10 re f".to_vec(),
        ));
        refuse_text(&doc).unwrap();
    }
}
