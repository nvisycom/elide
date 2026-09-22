//! Text redaction (glyph deletion): deleting the glyphs of detected spans from
//! a composite (Type0/Identity-H, CID) subset font, the case a naive
//! text-operator rewrite corrupts, and the recursive document-sanitise pass
//! that deletes structures retaining copies of text, including objects nested
//! below an annotation.

use elide_pdf::Pdf;
use elide_pdf::redact::Detection;

/// A one-page PDF whose text is drawn with an embedded TrueType **subset** in a
/// Type0/Identity-H (CID) font, the encoding that defeats whole-operand text
/// rewriting. It reads `"Contact alice@example.com at the office."` then
/// `"Keep this line intact."`.
const CID_FONT: &[u8] = include_bytes!("testdata/cid_font.pdf");

fn extracted(pdf_bytes: &[u8]) -> String {
    Pdf::open(pdf_bytes)
        .unwrap()
        .extract()
        .blocks
        .iter()
        .map(|b| b.text.to_string())
        .collect()
}

/// Char spans on any page whose text equals `needle`.
fn spans_for(pdf: &Pdf, needle: &str) -> Vec<Detection> {
    let mut out = Vec::new();
    for block in pdf.extract().blocks {
        let text = block.text.as_str();
        let mut from = 0;
        while let Some(pos) = text[from..].find(needle) {
            let byte_at = from + pos;
            let start = text[..byte_at].chars().count();
            out.push(Detection::new(
                block.page,
                start,
                start + needle.chars().count(),
            ));
            from = byte_at + needle.len();
        }
    }
    out
}

#[test]
fn deletes_a_detected_span_in_a_cid_subset_font() {
    let pdf = Pdf::open(CID_FONT).unwrap();
    let detections = spans_for(&pdf, "alice@example.com");
    assert!(
        !detections.is_empty(),
        "the fixture should carry the target"
    );

    let out = pdf.redact_text(&detections).unwrap();
    let text = extracted(&out);

    // The target is gone; the surrounding text (same CID font) is intact and
    // uncorrupted, deletion never re-encodes, so no glyph is mangled.
    assert!(!text.contains("alice@example.com"), "target survived");
    assert!(text.contains("Contact"), "leading context lost");
    assert!(text.contains("office"), "trailing context lost");
    assert!(text.contains("Keep this line intact"), "second line lost");
}

#[test]
fn sanitize_deletes_a_nested_annotation_subtree() {
    use lopdf::content::{Content, Operation};
    use lopdf::{Document, Object, Stream, dictionary};

    // A page with a link annotation whose appearance stream (/AP -> /N)
    // references a further object holding secret text, a grandchild of the
    // annotation. A one-level GC would leave that grandchild orphaned.
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();

    let font_id = doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Courier",
    });
    let resources_id = doc.add_object(dictionary! { "Font" => dictionary! { "F1" => font_id } });
    let body = Content {
        operations: vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 12.into()]),
            Operation::new("Td", vec![72.into(), 700.into()]),
            Operation::new("Tj", vec![Object::string_literal("Visible body")]),
            Operation::new("ET", vec![]),
        ],
    };
    let content_id = doc.add_object(Stream::new(dictionary! {}, body.encode().unwrap()));

    // Grandchild: a nested resource holding a secret string.
    let nested_id = doc.add_object(dictionary! {
        "Secret" => Object::string_literal("GRANDCHILD-SECRET"),
    });
    // The annotation's appearance stream references the grandchild.
    let ap_stream = Stream::new(
        dictionary! { "Type" => "XObject", "Subtype" => "Form", "Nested" => nested_id },
        b"q Q".to_vec(),
    );
    let ap_id = doc.add_object(ap_stream);
    let annot_id = doc.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Link",
        "Rect" => vec![0.into(), 0.into(), 10.into(), 10.into()],
        "AP" => dictionary! { "N" => Object::Reference(ap_id) },
    });
    let page_id = doc.add_object(dictionary! {
        "Type" => "Page", "Parent" => pages_id,
        "Contents" => content_id, "Resources" => resources_id,
        "Annots" => vec![annot_id.into()],
    });
    let pages = dictionary! {
        "Type" => "Pages", "Kids" => vec![page_id.into()], "Count" => 1,
        "MediaBox" => vec![0.into(), 0.into(), 200.into(), 200.into()],
    };
    doc.objects.insert(pages_id, Object::Dictionary(pages));
    let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog_id);
    let mut pdf = Vec::new();
    doc.save_to(&mut pdf).unwrap();

    // Sanity: the grandchild secret is in the source.
    assert!(String::from_utf8_lossy(&pdf).contains("GRANDCHILD-SECRET"));

    // With no detections, sanitize still runs; isolate its behaviour.
    let out = Pdf::open(&pdf).unwrap().redact_text(&[]).unwrap();

    // The whole annotation subtree, including the grandchild, is gone.
    assert!(
        !String::from_utf8_lossy(&out).contains("GRANDCHILD-SECRET"),
        "nested annotation object survived sanitize"
    );
    // The visible body survives.
    assert!(extracted(&out).contains("Visible body"));
}

/// A one-page PDF whose text is one `TJ` array interleaving strings with
/// non-zero kerning adjustments: `[(Sec) -40 (ret) -55 ( Agent)]`, drawn in a
/// simple Courier font (one byte per glyph). Reads `"Secret Agent"`.
fn tj_with_kerning() -> Vec<u8> {
    use lopdf::content::{Content, Operation};
    use lopdf::{Document, Object, Stream, dictionary};

    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let font_id = doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Courier",
    });
    let resources_id = doc.add_object(dictionary! { "Font" => dictionary! { "F1" => font_id } });
    let body = Content {
        operations: vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 12.into()]),
            Operation::new("Td", vec![72.into(), 700.into()]),
            Operation::new(
                "TJ",
                vec![Object::Array(vec![
                    Object::string_literal("Sec"),
                    (-40).into(),
                    Object::string_literal("ret"),
                    (-55).into(),
                    Object::string_literal(" Agent"),
                ])],
            ),
            Operation::new("ET", vec![]),
        ],
    };
    let content_id = doc.add_object(Stream::new(dictionary! {}, body.encode().unwrap()));
    let page_id = doc.add_object(dictionary! {
        "Type" => "Page", "Parent" => pages_id,
        "Contents" => content_id, "Resources" => resources_id,
    });
    let pages = dictionary! {
        "Type" => "Pages", "Kids" => vec![page_id.into()], "Count" => 1,
        "MediaBox" => vec![0.into(), 0.into(), 200.into(), 200.into()],
    };
    doc.objects.insert(pages_id, Object::Dictionary(pages));
    let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog_id);
    let mut pdf = Vec::new();
    doc.save_to(&mut pdf).unwrap();
    pdf
}

/// End to end: redacting a word drawn by a kerned `TJ` array removes the target
/// text while the survivor stays intact and readable. The precise assertion that
/// no non-zero position adjustment survives around the deleted glyphs lives as a
/// unit test on `apply_deletions` (it needs the in-crate content-stream view).
#[test]
fn redacts_a_span_drawn_by_a_kerned_tj_array() {
    let pdf = tj_with_kerning();
    let doc = Pdf::open(&pdf).unwrap();
    let detections = spans_for(&doc, "Secret");
    assert!(!detections.is_empty(), "fixture should carry the target");

    let out = doc.redact_text(&detections).unwrap();
    let text = extracted(&out);
    assert!(!text.contains("Secret"), "target survived");
    assert!(text.contains("Agent"), "survivor lost");
}

/// A page drawing text with the `'` (move-to-next-line-and-show) and `"`
/// (set-spacing-and-show) operators. Their text must be located and redactable,
/// not silently left in, `'` shows its whole operand, `"` shows only its third.
#[test]
fn redacts_text_drawn_by_quote_operators() {
    use lopdf::content::{Content, Operation};
    use lopdf::{Document, Object, Stream, dictionary};

    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let font_id = doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Courier",
    });
    let resources_id = doc.add_object(dictionary! { "Font" => dictionary! { "F1" => font_id } });
    let body = Content {
        operations: vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 12.into()]),
            Operation::new("Td", vec![72.into(), 700.into()]),
            // `'`: single string operand.
            Operation::new("'", vec![Object::string_literal("alpha SECRET1 beta")]),
            // `"`: `aw ac string` — only the third operand is shown.
            Operation::new(
                "\"",
                vec![
                    0.into(),
                    0.into(),
                    Object::string_literal("gamma SECRET2 delta"),
                ],
            ),
            Operation::new("ET", vec![]),
        ],
    };
    let content_id = doc.add_object(Stream::new(dictionary! {}, body.encode().unwrap()));
    let page_id = doc.add_object(dictionary! {
        "Type" => "Page", "Parent" => pages_id,
        "Contents" => content_id, "Resources" => resources_id,
    });
    let pages = dictionary! {
        "Type" => "Pages", "Kids" => vec![page_id.into()], "Count" => 1,
        "MediaBox" => vec![0.into(), 0.into(), 300.into(), 200.into()],
    };
    doc.objects.insert(pages_id, Object::Dictionary(pages));
    let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog_id);
    let mut pdf = Vec::new();
    doc.save_to(&mut pdf).unwrap();

    let opened = Pdf::open(&pdf).unwrap();
    // Both secrets are located (proving `'` and `"` text is walked).
    let mut dets = spans_for(&opened, "SECRET1");
    dets.extend(spans_for(&opened, "SECRET2"));
    assert_eq!(dets.len(), 2, "both quote-operator spans should be found");

    let out = opened.redact_text(&dets).unwrap();
    let text = extracted(&out);
    assert!(!text.contains("SECRET1"), "` ' ` text not redacted");
    assert!(!text.contains("SECRET2"), "` \" ` text not redacted");
    assert!(
        text.contains("alpha") && text.contains("beta"),
        "' context lost"
    );
    assert!(
        text.contains("gamma") && text.contains("delta"),
        "\" context lost"
    );
}

/// Optional-content (layer) machinery is stripped: the catalog's
/// `/OCProperties` configuration and its OCG dictionaries are removed, and the
/// `/OC` membership marks on content objects are cleared, so nothing survives
/// gated behind a hidden layer.
#[test]
fn sanitize_strips_optional_content_layers() {
    use lopdf::{Document, Object, Stream, dictionary};

    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let content_id = doc.add_object(Stream::new(dictionary! {}, b"BT ET".to_vec()));

    // An optional-content group, named so we can find it in the output bytes.
    let ocg_id = doc.add_object(dictionary! {
        "Type" => "OCG", "Name" => Object::string_literal("HIDDEN-LAYER"),
    });
    // A Form XObject gated to that layer via `/OC`.
    let form_id = doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form",
            "OC" => Object::Reference(ocg_id),
            "BBox" => vec![0.into(), 0.into(), 10.into(), 10.into()],
        },
        b"q Q".to_vec(),
    ));
    // A second OCG reachable only through the page's `/Resources /Properties`,
    // the marked-content path (`/OC /MC0 BDC ... EMC`). A scrub that only
    // clears `/OC` marks would leave this one reachable and retained.
    let mc_ocg_id = doc.add_object(dictionary! {
        "Type" => "OCG", "Name" => Object::string_literal("MARKED-LAYER"),
    });
    let resources_id = doc.add_object(dictionary! {
        "XObject" => dictionary! { "Fm0" => Object::Reference(form_id) },
        "Properties" => dictionary! { "MC0" => Object::Reference(mc_ocg_id) },
    });
    let page_id = doc.add_object(dictionary! {
        "Type" => "Page", "Parent" => pages_id,
        "Contents" => content_id, "Resources" => resources_id,
    });
    let pages = dictionary! {
        "Type" => "Pages", "Kids" => vec![page_id.into()], "Count" => 1,
        "MediaBox" => vec![0.into(), 0.into(), 100.into(), 100.into()],
    };
    doc.objects.insert(pages_id, Object::Dictionary(pages));
    let catalog_id = doc.add_object(dictionary! {
        "Type" => "Catalog", "Pages" => pages_id,
        "OCProperties" => dictionary! {
            "OCGs" => vec![Object::Reference(ocg_id), Object::Reference(mc_ocg_id)],
            "D" => dictionary! {
                "ON" => vec![Object::Reference(ocg_id), Object::Reference(mc_ocg_id)],
            },
        },
    });
    doc.trailer.set("Root", catalog_id);
    let mut pdf = Vec::new();
    doc.save_to(&mut pdf).unwrap();

    // Sanity: the source has both OCGs and reports them in the inventory.
    assert!(String::from_utf8_lossy(&pdf).contains("HIDDEN-LAYER"));
    assert!(String::from_utf8_lossy(&pdf).contains("MARKED-LAYER"));
    assert_eq!(
        Pdf::open(&pdf)
            .unwrap()
            .inspect()
            .unwrap()
            .risks
            .optional_content_group_count,
        2
    );

    let out = Pdf::open(&pdf).unwrap().redact_text(&[]).unwrap();

    // Both OCG objects are gone: the `/OC`-marked one and the one reachable
    // only through `/Resources /Properties`.
    assert!(
        !String::from_utf8_lossy(&out).contains("HIDDEN-LAYER"),
        "OC-marked OCG survived"
    );
    assert!(
        !String::from_utf8_lossy(&out).contains("MARKED-LAYER"),
        "marked-content OCG (via /Properties) survived"
    );
    let after = Pdf::open(&out).unwrap().inspect().unwrap();
    assert_eq!(after.risks.optional_content_group_count, 0);
    // The `/OC` mark on the surviving form is cleared.
    assert!(
        !String::from_utf8_lossy(&out).contains("/OC "),
        "an /OC membership mark survived"
    );
}

/// A one-page PDF whose PII text is drawn INSIDE a Form XObject invoked with
/// `Do` from the page content. The page content itself draws only a marker; the
/// XObject's own content stream holds `Tj (Contact bob@corp.com now)`.
fn form_xobject_with_text() -> Vec<u8> {
    use lopdf::content::{Content, Operation};
    use lopdf::{Document, Object, Stream, dictionary};

    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let font_id = doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Courier",
    });
    // The Form XObject's own content + resources.
    let form_body = Content {
        operations: vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 12.into()]),
            Operation::new("Td", vec![10.into(), 10.into()]),
            Operation::new(
                "Tj",
                vec![Object::string_literal("Contact bob@corp.com now")],
            ),
            Operation::new("ET", vec![]),
        ],
    };
    let form_resources = doc.add_object(dictionary! {
        "Font" => dictionary! { "F1" => font_id },
    });
    let form_id = doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form",
            "BBox" => vec![0.into(), 0.into(), 200.into(), 50.into()],
            "Resources" => Object::Reference(form_resources),
        },
        form_body.encode().unwrap(),
    ));
    // The page draws a marker of its own, then the form via `Do`.
    let page_body = Content {
        operations: vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 12.into()]),
            Operation::new("Td", vec![72.into(), 700.into()]),
            Operation::new("Tj", vec![Object::string_literal("PAGEMARK")]),
            Operation::new("ET", vec![]),
            Operation::new("Do", vec!["Fm0".into()]),
        ],
    };
    let content_id = doc.add_object(Stream::new(dictionary! {}, page_body.encode().unwrap()));
    let resources_id = doc.add_object(dictionary! {
        "Font" => dictionary! { "F1" => font_id },
        "XObject" => dictionary! { "Fm0" => Object::Reference(form_id) },
    });
    let page_id = doc.add_object(dictionary! {
        "Type" => "Page", "Parent" => pages_id,
        "Contents" => content_id, "Resources" => resources_id,
    });
    let pages = dictionary! {
        "Type" => "Pages", "Kids" => vec![page_id.into()], "Count" => 1,
        "MediaBox" => vec![0.into(), 0.into(), 300.into(), 800.into()],
    };
    doc.objects.insert(pages_id, Object::Dictionary(pages));
    let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog_id);
    let mut pdf = Vec::new();
    doc.save_to(&mut pdf).unwrap();
    pdf
}

/// A Form XObject that draws text WITHOUT selecting its own font: it inherits the
/// font the caller selected with `Tf` before the `Do`. The form's content shares
/// the caller's resources (it has no `/Resources` of its own), so `F1` resolves
/// in the parent scope.
fn form_xobject_inheriting_font() -> Vec<u8> {
    use lopdf::content::{Content, Operation};
    use lopdf::{Document, Object, Stream, dictionary};

    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let font_id = doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Courier",
    });
    // The form draws text but never issues its own `Tf`; it relies on the font
    // the page selected before the `Do`. It declares no `/Resources`, so the
    // page's resources stand in.
    let form_body = Content {
        operations: vec![
            Operation::new("BT", vec![]),
            Operation::new("Td", vec![10.into(), 10.into()]),
            Operation::new(
                "Tj",
                vec![Object::string_literal("Contact bob@corp.com now")],
            ),
            Operation::new("ET", vec![]),
        ],
    };
    let form_id = doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form",
            "BBox" => vec![0.into(), 0.into(), 200.into(), 50.into()],
        },
        form_body.encode().unwrap(),
    ));
    // The page selects F1, then draws the form via `Do`.
    let page_body = Content {
        operations: vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 12.into()]),
            Operation::new("Td", vec![72.into(), 700.into()]),
            Operation::new("Tj", vec![Object::string_literal("PAGEMARK")]),
            Operation::new("ET", vec![]),
            Operation::new("Do", vec!["Fm0".into()]),
        ],
    };
    let content_id = doc.add_object(Stream::new(dictionary! {}, page_body.encode().unwrap()));
    let resources_id = doc.add_object(dictionary! {
        "Font" => dictionary! { "F1" => font_id },
        "XObject" => dictionary! { "Fm0" => Object::Reference(form_id) },
    });
    let page_id = doc.add_object(dictionary! {
        "Type" => "Page", "Parent" => pages_id,
        "Contents" => content_id, "Resources" => resources_id,
    });
    let pages = dictionary! {
        "Type" => "Pages", "Kids" => vec![page_id.into()], "Count" => 1,
        "MediaBox" => vec![0.into(), 0.into(), 300.into(), 800.into()],
    };
    doc.objects.insert(pages_id, Object::Dictionary(pages));
    let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog_id);
    let mut pdf = Vec::new();
    doc.save_to(&mut pdf).unwrap();
    pdf
}

/// Text a Form XObject draws with the caller's inherited font (no `Tf` of its
/// own) is walked and redactable, not silently skipped. Before the walker
/// carried the inherited font into the form, `current` was `None` there and the
/// text was dropped, so it was neither extracted nor redacted, a leak.
#[test]
fn redacts_text_drawn_with_an_inherited_font() {
    let pdf = form_xobject_inheriting_font();
    let doc = Pdf::open(&pdf).unwrap();

    let joined: String = doc
        .extract()
        .blocks
        .into_iter()
        .map(|b| b.text.to_string())
        .collect();
    assert!(
        joined.contains("bob@corp.com"),
        "inherited-font XObject text not walked"
    );

    let dets = spans_for(&doc, "bob@corp.com");
    assert!(!dets.is_empty(), "target not located");
    let out = doc.redact_text(&dets).unwrap();

    let reopened = Pdf::open(&out).unwrap();
    let walked: String = reopened
        .extract()
        .blocks
        .into_iter()
        .map(|b| b.text.to_string())
        .collect();
    assert!(
        !walked.contains("bob@corp.com"),
        "inherited-font PII survived"
    );
    assert!(walked.contains("PAGEMARK"), "page text lost");
    assert!(
        !String::from_utf8_lossy(&out).contains("bob@corp.com"),
        "inherited-font PII still in raw bytes"
    );
}

/// Text drawn inside a `Do`-invoked Form XObject is located and redactable: the
/// walker recurses into the XObject's content stream, so its PII is found and
/// deleted from that stream, while the page's own text survives.
#[test]
fn redacts_text_inside_a_form_xobject() {
    let pdf = form_xobject_with_text();
    let doc = Pdf::open(&pdf).unwrap();

    // The XObject text appears in the extraction (proving the walker recursed).
    let joined: String = doc
        .extract()
        .blocks
        .into_iter()
        .map(|b| b.text.to_string())
        .collect();
    assert!(joined.contains("bob@corp.com"), "XObject text not walked");
    assert!(joined.contains("PAGEMARK"), "page text missing");

    let dets = spans_for(&doc, "bob@corp.com");
    assert!(!dets.is_empty(), "target not located");
    let out = doc.redact_text(&dets).unwrap();

    // Re-walk the output (our walker recurses into the XObject; lopdf's own
    // extraction does not): the PII is gone, its XObject context and the page
    // text survive, and the PII is absent from the raw output bytes.
    let reopened = Pdf::open(&out).unwrap();
    let walked: String = reopened
        .extract()
        .blocks
        .into_iter()
        .map(|b| b.text.to_string())
        .collect();
    assert!(!walked.contains("bob@corp.com"), "XObject PII survived");
    assert!(
        walked.contains("Contact") && walked.contains("now"),
        "XObject context lost"
    );
    assert!(walked.contains("PAGEMARK"), "page text lost");
    assert!(
        !String::from_utf8_lossy(&out).contains("bob@corp.com"),
        "XObject PII still in raw bytes"
    );
}

/// A Form XObject that draws itself (a `Do` cycle) must not hang or crash: the
/// walker's cycle guard stops re-entry.
#[test]
fn a_form_xobject_do_cycle_terminates() {
    use lopdf::content::{Content, Operation};
    use lopdf::{Document, Object, Stream, dictionary};

    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let form_id = doc.new_object_id();
    // The form draws itself via `Do Fm0`, with its own resources naming Fm0.
    let form_body = Content {
        operations: vec![Operation::new("Do", vec!["Fm0".into()])],
    };
    let form_resources = doc.add_object(dictionary! {
        "XObject" => dictionary! { "Fm0" => Object::Reference(form_id) },
    });
    doc.objects.insert(
        form_id,
        Object::Stream(Stream::new(
            dictionary! {
                "Type" => "XObject", "Subtype" => "Form",
                "BBox" => vec![0.into(), 0.into(), 10.into(), 10.into()],
                "Resources" => Object::Reference(form_resources),
            },
            form_body.encode().unwrap(),
        )),
    );
    let content_id = doc.add_object(Stream::new(
        dictionary! {},
        Content {
            operations: vec![Operation::new("Do", vec!["Fm0".into()])],
        }
        .encode()
        .unwrap(),
    ));
    let resources_id = doc.add_object(dictionary! {
        "XObject" => dictionary! { "Fm0" => Object::Reference(form_id) },
    });
    let page_id = doc.add_object(dictionary! {
        "Type" => "Page", "Parent" => pages_id,
        "Contents" => content_id, "Resources" => resources_id,
    });
    let pages = dictionary! {
        "Type" => "Pages", "Kids" => vec![page_id.into()], "Count" => 1,
        "MediaBox" => vec![0.into(), 0.into(), 100.into(), 100.into()],
    };
    doc.objects.insert(pages_id, Object::Dictionary(pages));
    let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog_id);
    let mut pdf = Vec::new();
    doc.save_to(&mut pdf).unwrap();

    // Terminates (no hang, no stack overflow) and yields text (empty here).
    let opened = Pdf::open(&pdf).unwrap();
    let _ = opened.extract();
    let _ = opened.redact_text(&[]).unwrap();
}

/// A `/ToUnicode` CMap shared by page text and Form-XObject text. Deleting a
/// code the (redacted) page draws must scrub only codes NOT still drawn by the
/// surviving XObject text: the surviving-code inventory covers XObject glyphs,
/// so a code the XObject still uses is spared and its text stays extractable.
#[test]
fn a_code_kept_by_surviving_xobject_text_is_spared() {
    use lopdf::content::{Content, Operation};
    use lopdf::{Document, Object, Stream, dictionary};

    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();

    // A ToUnicode CMap mapping code 0x41 'A' and 0x42 'B'. Both simple-font
    // codes; the CMap is shared by the page's font and the XObject's font.
    let cmap = b"/CIDInit /ProcSet findresource begin 12 dict begin begincmap\n\
1 begincodespacerange <00> <ff> endcodespacerange\n\
2 beginbfchar\n<41> <0041>\n<42> <0042>\nendbfchar\nendcmap end end\n";
    let cmap_id = doc.add_object(Stream::new(dictionary! {}, cmap.to_vec()));
    let font_id = doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Courier",
        "ToUnicode" => Object::Reference(cmap_id),
    });

    // The XObject draws "B" (code 0x42) and survives.
    let form_body = Content {
        operations: vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 12.into()]),
            Operation::new(
                "Tj",
                vec![Object::String(vec![0x42], lopdf::StringFormat::Literal)],
            ),
            Operation::new("ET", vec![]),
        ],
    };
    let form_resources = doc.add_object(dictionary! { "Font" => dictionary! { "F1" => font_id } });
    let form_id = doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form",
            "BBox" => vec![0.into(), 0.into(), 50.into(), 50.into()],
            "Resources" => Object::Reference(form_resources),
        },
        form_body.encode().unwrap(),
    ));

    // The page draws "A" (code 0x41, redacted) then invokes the form.
    let page_body = Content {
        operations: vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 12.into()]),
            Operation::new("Td", vec![72.into(), 700.into()]),
            Operation::new(
                "Tj",
                vec![Object::String(vec![0x41], lopdf::StringFormat::Literal)],
            ),
            Operation::new("ET", vec![]),
            Operation::new("Do", vec!["Fm0".into()]),
        ],
    };
    let content_id = doc.add_object(Stream::new(dictionary! {}, page_body.encode().unwrap()));
    let resources_id = doc.add_object(dictionary! {
        "Font" => dictionary! { "F1" => font_id },
        "XObject" => dictionary! { "Fm0" => Object::Reference(form_id) },
    });
    let page_id = doc.add_object(dictionary! {
        "Type" => "Page", "Parent" => pages_id,
        "Contents" => content_id, "Resources" => resources_id,
    });
    let pages = dictionary! {
        "Type" => "Pages", "Kids" => vec![page_id.into()], "Count" => 1,
        "MediaBox" => vec![0.into(), 0.into(), 300.into(), 800.into()],
    };
    doc.objects.insert(pages_id, Object::Dictionary(pages));
    let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog_id);
    let mut pdf = Vec::new();
    doc.save_to(&mut pdf).unwrap();

    // Redact the page's "A".
    let opened = Pdf::open(&pdf).unwrap();
    let dets = spans_for(&opened, "A");
    assert!(!dets.is_empty(), "page 'A' not located");
    let out = opened.redact_text(&dets).unwrap();

    // The shared `/ToUnicode` CMap keeps the `<42>` ('B') entry the surviving
    // XObject text still needs, and drops the `<41>` ('A') entry the redacted
    // page drew: the surviving-code inventory counted the XObject's glyph, so
    // its mapping was spared. (Asserted on the CMap bytes: the code->Unicode
    // table is exactly what the scrub edits.)
    let raw = String::from_utf8_lossy(&out);
    assert!(
        raw.contains("<42>"),
        "surviving XObject's CMap entry was scrubbed"
    );
    assert!(
        !raw.contains("<41>"),
        "redacted page's CMap entry was not scrubbed"
    );
}

/// A Form XObject drawn by two pages (a shared header/footer form) with PII in
/// its own stream. Redacting spans located on both pages must delete every
/// targeted glyph from the single shared stream, not just those from the last
/// page written. The redactor accumulates deletions across all pages before
/// rewriting each physical stream once; a per-page decode-and-write would let
/// the second page overwrite the first page's edit with the pristine bytes,
/// resurrecting the redacted text.
#[test]
fn a_form_xobject_shared_by_two_pages_keeps_every_page_deletion() {
    use lopdf::content::{Content, Operation};
    use lopdf::{Document, Object, Stream, dictionary};

    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let font_id = doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Courier",
    });
    // One form, drawing two distinct emails, shared by both pages.
    let form_body = Content {
        operations: vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 12.into()]),
            Operation::new("Td", vec![10.into(), 10.into()]),
            Operation::new(
                "Tj",
                vec![Object::string_literal("alice@x.com and bob@y.com")],
            ),
            Operation::new("ET", vec![]),
        ],
    };
    let form_resources = doc.add_object(dictionary! { "Font" => dictionary! { "F1" => font_id } });
    let form_id = doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form",
            "BBox" => vec![0.into(), 0.into(), 200.into(), 50.into()],
            "Resources" => Object::Reference(form_resources),
        },
        form_body.encode().unwrap(),
    ));

    // Two pages, each drawing the same form via `Do`.
    let resources = dictionary! {
        "Font" => dictionary! { "F1" => font_id },
        "XObject" => dictionary! { "Fm0" => Object::Reference(form_id) },
    };
    let mut page_ids = Vec::new();
    for _ in 0..2 {
        let body = Content {
            operations: vec![Operation::new("Do", vec!["Fm0".into()])],
        };
        let content_id = doc.add_object(Stream::new(dictionary! {}, body.encode().unwrap()));
        let resources_id = doc.add_object(resources.clone());
        page_ids.push(
            doc.add_object(dictionary! {
                "Type" => "Page", "Parent" => pages_id,
                "Contents" => content_id, "Resources" => resources_id,
            })
            .into(),
        );
    }
    let pages = dictionary! {
        "Type" => "Pages", "Kids" => page_ids, "Count" => 2,
        "MediaBox" => vec![0.into(), 0.into(), 300.into(), 100.into()],
    };
    doc.objects.insert(pages_id, Object::Dictionary(pages));
    let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog_id);
    let mut pdf = Vec::new();
    doc.save_to(&mut pdf).unwrap();

    // Both emails appear once per page (the form is walked under each), so a
    // detection lands on both pages, each resolving to the same shared stream.
    let opened = Pdf::open(&pdf).unwrap();
    let mut dets = spans_for(&opened, "alice@x.com");
    dets.extend(spans_for(&opened, "bob@y.com"));
    assert!(
        dets.len() >= 4,
        "both targets should be found on both pages"
    );

    let out = opened.redact_text(&dets).unwrap();

    // Neither email survives, in the re-extracted text or the raw bytes: the
    // shared stream kept every page's deletion.
    let text = extracted(&out);
    assert!(
        !text.contains("alice@x.com"),
        "first-page deletion was lost"
    );
    assert!(!text.contains("bob@y.com"), "second-page deletion was lost");
    let raw = String::from_utf8_lossy(&out);
    assert!(
        !raw.contains("alice@x.com"),
        "first target still in raw bytes"
    );
    assert!(
        !raw.contains("bob@y.com"),
        "second target still in raw bytes"
    );
}

/// Two pages with SEPARATE content streams whose text sits at the same
/// operation/operand indexes. A detection on each page must delete only that
/// page's target and leave the other page's text (at the identical index)
/// untouched. Deletions are scoped by physical stream, not by operand address
/// alone, so page 1's ranges never drain page 2's bytes.
#[test]
fn distinct_page_streams_with_matching_indexes_do_not_cross_contaminate() {
    use lopdf::content::{Content, Operation};
    use lopdf::{Document, Object, Stream, dictionary};

    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let font_id = doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Courier",
    });

    // Each page's single Tj sits at the same operation index (op 3) but in its
    // own Contents object, and draws a different string of the same length.
    let page_body = |text: &str| Content {
        operations: vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 12.into()]),
            Operation::new("Td", vec![72.into(), 700.into()]),
            Operation::new("Tj", vec![Object::string_literal(text)]),
            Operation::new("ET", vec![]),
        ],
    };
    let mut page_ids = Vec::new();
    for text in ["alice@x.com KEEP1", "bob@y.com   KEEP2"] {
        let content_id = doc.add_object(Stream::new(
            dictionary! {},
            page_body(text).encode().unwrap(),
        ));
        let resources_id =
            doc.add_object(dictionary! { "Font" => dictionary! { "F1" => font_id } });
        page_ids.push(
            doc.add_object(dictionary! {
                "Type" => "Page", "Parent" => pages_id,
                "Contents" => content_id, "Resources" => resources_id,
            })
            .into(),
        );
    }
    let pages = dictionary! {
        "Type" => "Pages", "Kids" => page_ids, "Count" => 2,
        "MediaBox" => vec![0.into(), 0.into(), 300.into(), 800.into()],
    };
    doc.objects.insert(pages_id, Object::Dictionary(pages));
    let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog_id);
    let mut pdf = Vec::new();
    doc.save_to(&mut pdf).unwrap();

    // Redact one email per page (each on its own page/stream, same op index).
    let opened = Pdf::open(&pdf).unwrap();
    let mut dets = spans_for(&opened, "alice@x.com");
    dets.extend(spans_for(&opened, "bob@y.com"));
    assert_eq!(dets.len(), 2, "one target on each page");

    let out = opened.redact_text(&dets).unwrap();
    let text = extracted(&out);

    // Both targets gone; both pages' non-detected text survives intact, neither
    // page's deletion drained the other page's same-index bytes.
    assert!(!text.contains("alice@x.com"), "page 1 target survived");
    assert!(!text.contains("bob@y.com"), "page 2 target survived");
    assert!(
        text.contains("KEEP1"),
        "page 1 surviving text was corrupted"
    );
    assert!(
        text.contains("KEEP2"),
        "page 2 surviving text was corrupted"
    );
}
