//! The rebuild's tests.

use elide_core::ErrorKind;
use lopdf::content::{Content, Operation};
use lopdf::{IncrementalDocument, Stream, StringFormat, dictionary};

use super::check::check;
use super::*;
use crate::content::attachment::Tree;
use crate::content::text::{Detection, Text, TextBlock, TextRedaction};
use crate::fixtures::{
    DocBuilder, FontFile, Page, Sfnt, Type1, blank, embedding, encoded, shown, to_unicode_cmap,
    type1, with_attachments, with_pages,
};
use crate::pdf::reader::DictionaryExt;

/// A one-page document: Helvetica as `/F1`, the page drawing `operations`,
/// with `resources` added to its resource dictionary and `page` entries
/// set on the page.
fn document(
    operations: Vec<Operation>,
    resources: Dictionary,
    page: Dictionary,
) -> (Document, ObjectId) {
    let mut pdf = DocBuilder::new();
    let mut font = type1("Helvetica");
    font.set("Secret", Object::string_literal("Jane Roe"));
    let font = pdf.add(font);
    pdf.page(
        Page::new()
            .font("F1", font)
            .resources(resources)
            .ops(operations)
            .size(200, 200)
            .entries(page),
    );
    let (doc, pages) = pdf.finish();
    (doc, pages[0])
}

/// `doc` rebuilt, saved, and read back, with what the rebuild recorded.
fn rebuilt(doc: &Document) -> (Document, StrippedContent) {
    let (bytes, stripped) = saved(doc);
    let out = Document::load_mem(&bytes).unwrap();
    normal::verify(&out).unwrap();
    (out, stripped)
}

/// `doc` rebuilt and saved, with what the rebuild recorded.
fn saved(doc: &Document) -> (Vec<u8>, StrippedContent) {
    let (document, stripped) = rebuild(doc).unwrap();
    let pdf = Edit::of(document).finish().unwrap();
    (pdf.bytes().to_vec(), stripped)
}

/// `pdf` with the glyphs of `detections`, located through `blocks`,
/// deleted.
fn deleted(pdf: &Pdf, blocks: &[TextBlock], detections: &[Detection]) -> Pdf {
    let mut edit = pdf.edit();
    edit.redact(TextRedaction {
        source: pdf,
        blocks,
        detections,
    })
    .unwrap();
    edit.finish().unwrap()
}

fn count(stripped: &StrippedContent, content: HiddenContent) -> usize {
    stripped
        .iter()
        .filter(|entry| entry.content == content)
        .map(|entry| entry.count)
        .sum()
}

/// Every string the rebuilt document holds, its streams decoded.
fn text_of(doc: &Document) -> String {
    let mut out = String::new();
    for object in doc.objects.values() {
        if let Object::Stream(stream) = object {
            let bytes = stream
                .decompressed_content()
                .unwrap_or(stream.content.clone());
            out.push_str(&String::from_utf8_lossy(&bytes));
        }
        out.push_str(&format!("{object:?}"));
    }
    out
}

fn page_dict(doc: &Document) -> &Dictionary {
    let id = doc.get_pages().into_values().next().unwrap();
    doc.get_dictionary(id).unwrap()
}

/// What the source held outside its pages is left behind, each item
/// recorded by what it was.
#[test]
fn hidden_structures_are_left_behind_and_recorded() {
    let (mut doc, _) = document(
        shown("Hello"),
        Dictionary::new(),
        dictionary! {
            "Thumb" => Object::string_literal("thumbnail"),
            "PieceInfo" => dictionary! { "Illustrator" => "private" },
            "Annots" => vec![Object::Dictionary(dictionary! {
                "Subtype" => "Text",
                "Contents" => Object::string_literal("ask Jane about the fee"),
            })],
        },
    );
    let xmp = doc.add_object(Stream::new(
        dictionary! { "Type" => "Metadata", "Subtype" => "XML" },
        b"<x:xmpmeta>Jane Roe</x:xmpmeta>".to_vec(),
    ));
    let script = doc.add_object(dictionary! { "S" => "JavaScript", "JS" => "app.alert(1)" });
    let catalog = doc.catalog_mut().unwrap();
    catalog.set("Metadata", xmp);
    catalog.set("OpenAction", script);
    catalog.set(
        "Outlines",
        dictionary! { "Title" => Object::string_literal("Jane") },
    );
    let info = doc.add_object(dictionary! {
        "Author" => Object::String(b"Jane Roe".to_vec(), StringFormat::Literal),
    });
    doc.trailer.set("Info", info);
    doc.trailer
        .set("ID", vec![Object::string_literal("fingerprint")]);
    doc.add_object(Object::string_literal("deleted paragraph"));

    let (out, stripped) = rebuilt(&doc);
    for content in [
        HiddenContent::Properties,
        HiddenContent::Action,
        HiddenContent::Outline,
        HiddenContent::DocumentId,
        HiddenContent::Thumbnail,
        HiddenContent::ApplicationData,
        HiddenContent::Annotation,
        HiddenContent::PriorRevision,
    ] {
        assert!(count(&stripped, content) > 0, "{content:?} unrecorded");
    }
    let text = text_of(&out);
    for leaked in [
        "Jane",
        "app.alert",
        "thumbnail",
        "Illustrator",
        "fingerprint",
        "deleted",
    ] {
        assert!(!text.contains(leaked), "{leaked} survived");
    }
    assert!(text.contains("Hello"));
    assert!(page_dict(&out).get(b"Annots").is_err());
}

/// A rebuild of a rebuild drops nothing: the output check holds.
#[test]
fn a_rebuild_is_its_own_rebuild() {
    let (doc, _) = document(shown("Hello"), Dictionary::new(), Dictionary::new());
    let (out, _) = rebuilt(&doc);
    check(&out).unwrap();
    assert!(check(&doc).is_err(), "the source font's extra key passed");
}

/// An operator whose operands are not what it takes is cut, so a string
/// among them never ships unread.
#[test]
fn an_operator_with_operands_of_another_shape_is_cut() {
    let mut ops = vec![
        Operation::new("w", vec![Object::string_literal("123-45-6789")]),
        Operation::new("w", vec![2.into()]),
    ];
    ops.extend(shown("Hello"));
    let (doc, _) = document(ops, Dictionary::new(), Dictionary::new());
    let (out, _) = rebuilt(&doc);
    let text = text_of(&out);
    assert!(!text.contains("123-45-6789"));
    assert!(text.contains("Hello") && text.contains("2 w"));
    check(&out).unwrap();
}

/// A copied image or mask is an image XObject, so the image pipeline
/// sees it, whatever its source said; a JPEG's one filter, given as an
/// array, is written as its name, as the normal form holds, and its
/// parameters, given as an array referred to, as their dictionary, with
/// the colour transform that decodes it.
#[test]
fn copied_images_are_image_xobjects_with_named_filters() {
    let (mut doc, page) = document(
        vec![Operation::new("Do", vec!["Im".into()])],
        Dictionary::new(),
        Dictionary::new(),
    );
    let mask = doc.add_object(Stream::new(
        dictionary! { "Width" => 8, "Height" => 1, "BitsPerComponent" => 8, "ColorSpace" => "DeviceGray" },
        vec![0; 8],
    ));
    let parameters = doc.add_object(vec![Object::Dictionary(dictionary! {
        "ColorTransform" => 0, "Note" => Object::string_literal("Jane Roe"),
    })]);
    let jpeg = doc.add_object(
        Stream::new(
            dictionary! {
                "Type" => "XObject", "Subtype" => "Image", "Width" => 8, "Height" => 8,
                "BitsPerComponent" => 8, "ColorSpace" => "DeviceRGB",
                "Filter" => vec![Object::from("DCTDecode")],
                "DecodeParms" => parameters,
                "SMask" => mask,
            },
            b"jpeg".to_vec(),
        )
        .with_compression(false),
    );
    doc.get_dictionary_mut(page)
        .unwrap()
        .get_mut(b"Resources")
        .and_then(Object::as_dict_mut)
        .unwrap()
        .set("XObject", dictionary! { "Im" => jpeg });
    let (out, _) = rebuilt(&doc);
    let Object::Stream(image) = resource(&out, b"XObject", b"Im") else {
        panic!("image");
    };
    assert_eq!(
        image.dict.get(b"Filter").unwrap(),
        &Object::from("DCTDecode")
    );
    let parameters = image
        .dict
        .get(b"DecodeParms")
        .and_then(Object::as_dict)
        .unwrap();
    assert_eq!(parameters.len(), 1);
    assert_eq!(
        parameters.get(b"ColorTransform").unwrap(),
        &Object::Integer(0)
    );
    let mask = out.get_object(image.dict.get(b"SMask").unwrap().as_reference().unwrap());
    let Ok(Object::Stream(mask)) = mask else {
        panic!("mask");
    };
    assert!(mask.dict.is_subtype(&out, b"Image"));
    // The note dropped is recorded: a rebuild that carries one again is
    // not passed as clean.
    check(&out).unwrap();
    let mut noted = out.clone();
    let image = noted
        .objects
        .values_mut()
        .find_map(|object| match object {
            Object::Stream(stream) if stream.dict.has(b"DecodeParms") => Some(stream),
            _ => None,
        })
        .unwrap();
    image
        .dict
        .get_mut(b"DecodeParms")
        .and_then(Object::as_dict_mut)
        .unwrap()
        .set("Note", Object::string_literal("Jane Roe"));
    assert!(check(&noted).is_err(), "the JPEG's note passed");
}

/// An image stored as raw samples is written as exactly the samples it
/// takes: bytes past them do not ship.
#[test]
fn bytes_past_an_images_samples_are_dropped() {
    let (mut doc, page) = document(
        vec![Operation::new("Do", vec!["Im".into()])],
        Dictionary::new(),
        Dictionary::new(),
    );
    let mask = doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Image", "Width" => 8, "Height" => 1,
            "ImageMask" => true,
        },
        [&[0x0F][..], b"tail-secret"].concat(),
    ));
    doc.get_dictionary_mut(page)
        .unwrap()
        .get_mut(b"Resources")
        .and_then(Object::as_dict_mut)
        .unwrap()
        .set("XObject", dictionary! { "Im" => mask });
    let (out, _) = rebuilt(&doc);
    let Object::Stream(image) = resource(&out, b"XObject", b"Im") else {
        panic!("image");
    };
    assert_eq!(image.decompressed_content().unwrap(), [0x0F]);
    assert!(!text_of(&out).contains("tail-secret"));
    check(&out).unwrap();
}

/// The rebuilt image under `Im` on the first page of `doc`, drawing it.
fn drawn_image(mut doc: Document, page: ObjectId, image: ObjectId) -> Vec<u8> {
    doc.get_dictionary_mut(page)
        .unwrap()
        .get_mut(b"Resources")
        .and_then(Object::as_dict_mut)
        .unwrap()
        .set("XObject", dictionary! { "Im" => image });
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).unwrap();
    let pdf = Pdf::open(&bytes).unwrap().read(Rebuild).unwrap().pdf;
    let Object::Stream(image) = resource(pdf.doc(), b"XObject", b"Im") else {
        panic!("image");
    };
    image.decompressed_content().unwrap()
}

/// The bits past a row's last sample are cleared, so nothing rides in
/// them.
#[test]
fn bits_past_a_rows_samples_are_cleared() {
    let (mut doc, page) = document(
        vec![Operation::new("Do", vec!["Im".into()])],
        Dictionary::new(),
        Dictionary::new(),
    );
    let mask = doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Image", "Width" => 3, "Height" => 2,
            "ImageMask" => true,
        },
        vec![0xFF, 0x5F],
    ));
    assert_eq!(drawn_image(doc, page, mask), [0xE0, 0x40]);
}

/// A stencil mask whose `/ImageMask` and `/Decode` are references is
/// still a stencil a codec reads, not removed.
#[test]
fn a_stencils_indirect_flags_are_followed() {
    let (mut doc, page) = document(
        vec![Operation::new("Do", vec!["Im".into()])],
        Dictionary::new(),
        Dictionary::new(),
    );
    let mask = doc.add_object(true);
    let decode = doc.add_object(vec![1.into(), 0.into()]);
    let image = doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Image", "Width" => 8, "Height" => 1,
            "ImageMask" => mask, "Decode" => decode,
        },
        vec![0x0F],
    ));
    assert_eq!(drawn_image(doc, page, image), [0x0F]);
}

/// An image whose filter, predictor parameters and width are references
/// decodes as one whose are direct.
#[test]
fn an_images_indirect_encoding_is_followed() {
    let (mut doc, page) = document(
        vec![Operation::new("Do", vec!["Im".into()])],
        Dictionary::new(),
        Dictionary::new(),
    );
    // Rows of two gray samples, each behind a PNG `None` predictor byte;
    // enough of them that the stream compresses.
    let mut image = Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Image", "Height" => 64,
            "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8,
        },
        [0, 10, 20].repeat(64),
    );
    image.compress().unwrap();
    assert!(image.dict.get(b"Filter").is_ok());
    let filter = doc.add_object(Object::from("FlateDecode"));
    let parms = doc.add_object(dictionary! { "Predictor" => 15, "Colors" => 1, "Columns" => 2 });
    let width = doc.add_object(2);
    image.dict.set("Filter", filter);
    image.dict.set("DecodeParms", parms);
    image.dict.set("Width", width);
    let image = doc.add_object(image);
    assert_eq!(drawn_image(doc, page, image), [10, 20].repeat(64));
}

/// Arrays referring many times to arrays expand past the source when
/// copied in place, so a copy past the budget is refused; one within it
/// is not.
#[test]
fn arrays_fanning_out_are_refused() {
    let mut doc = Document::with_version("1.7");
    let mut widths = doc.add_object(vec![Object::Integer(500); 8]);
    for _ in 0..3 {
        widths = doc.add_object(vec![Object::Reference(widths); 8]);
    }
    let widths = Object::Reference(widths);
    let copy = |budget: usize| {
        let mut builder = Builder::new(&doc);
        builder.copy_budget = budget;
        builder.copy(Kind::Value, &widths, At::top(&Scope::default()))
    };
    // Each reference and the array it refers to, at every level (1, 8,
    // 64 and 512 of them), and the 4096 numbers at the leaves.
    let values = 2 * (1 + 8 + 64 + 512) + 4096;
    let copied = copy(values * VALUE_COST)
        .unwrap()
        .expect("the array is copied");
    let leaves = |object: &Object| -> usize {
        fn count(object: &Object) -> usize {
            match object {
                Object::Array(items) => items.iter().map(count).sum(),
                _ => 1,
            }
        }
        count(object)
    };
    assert_eq!(leaves(&copied), 4096);
    let error = copy(values * VALUE_COST - 1).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::ResourceLimit);
}

/// Content that borrows its scope is rebuilt, and charged, once per
/// scope: a budget for one rebuild does not cover a second.
#[test]
fn each_rebuild_of_borrowing_content_is_charged() {
    let mut doc = Document::with_version("1.7");
    let form = doc.add_object(Stream::new(
        dictionary! { "Type" => "XObject", "Subtype" => "Form" },
        b"0 0 1 1 re f ".repeat(100),
    ));
    let form = Object::Reference(form);
    let (one, two) = (
        dictionary! { "ExtGState" => dictionary! {} },
        dictionary! { "Pattern" => dictionary! {} },
    );
    let (one, two) = (Scope::new([&one]), Scope::new([&two]));
    let used = {
        let mut builder = Builder::new(&doc);
        builder.copy(Kind::Form, &form, At::top(&one)).unwrap();
        MAX_COPIED_BYTES - builder.copy_budget
    };
    let mut builder = Builder::new(&doc);
    builder.copy_budget = used + used / 2;
    builder.copy(Kind::Form, &form, At::top(&one)).unwrap();
    let error = builder.copy(Kind::Form, &form, At::top(&two)).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::ResourceLimit);
}

/// Each page's rebuilt content is charged, so pages sharing one content
/// stream cost it once per page.
#[test]
fn each_pages_content_is_charged() {
    let mut doc = Document::with_version("1.7");
    let content = doc.add_object(Stream::new(dictionary! {}, b"0 0 1 1 re f ".repeat(1_000)));
    let pages = with_pages(
        &mut doc,
        [0, 1].map(|_| dictionary! { "Contents" => content, "Resources" => dictionary! {} }),
    );
    let parent = (9999, 0);
    let builder = || {
        let mut builder = Builder::new(&doc);
        for (n, &page) in pages.iter().enumerate() {
            builder.pages.insert(page, (10_000 + n as u32, 0));
        }
        builder
    };
    let used = {
        let mut builder = builder();
        builder.page(pages[0], parent).unwrap();
        MAX_COPIED_BYTES - builder.copy_budget
    };
    let mut builder = builder();
    builder.copy_budget = used + used / 2;
    builder.page(pages[0], parent).unwrap();
    let error = builder.page(pages[1], parent).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::ResourceLimit);
}

/// A string referenced many times costs its bytes each time it is copied.
#[test]
fn a_string_referred_to_again_and_again_is_charged_each_time() {
    let mut doc = Document::with_version("1.7");
    let text = doc.add_object(Object::string_literal(vec![b'x'; 1_000]));
    let array = Object::Array(vec![Object::Reference(text); 100]);
    let mut builder = Builder::new(&doc);
    builder.copy_budget = 50_000;
    let error = builder
        .copy(Kind::Label, &array, At::top(&Scope::default()))
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::ResourceLimit);
}

/// A value keeps only the scalars its kind can be: a string where a
/// number or name goes is dropped, so it can't carry text detection never
/// reads; a label keeps its string; a file path in a form's `/AF` is
/// recorded, and the files beside it are kept.
#[test]
fn scalars_are_kept_only_where_their_kind_can_be_one() {
    let doc = Document::with_version("1.7");
    let mut builder = Builder::new(&doc);
    let scope = Scope::default();
    let at = || At::top(&scope);
    let note = Object::string_literal("Jane Roe, 555-0142");
    assert_eq!(builder.copy(Kind::Value, &note, at()).unwrap(), None);
    assert_eq!(
        builder.copy(Kind::Font, &Object::Integer(1), at()).unwrap(),
        None
    );
    assert_eq!(
        builder.copy(Kind::Label, &note, at()).unwrap(),
        Some(note.clone())
    );
    assert_eq!(
        builder
            .copy(Kind::Value, &Object::Integer(7), at())
            .unwrap(),
        Some(Object::Integer(7))
    );

    let mut pdf = DocBuilder::new();
    let form = pdf.form(
        dictionary! {
            "AF" => vec![Object::string_literal("C:\\Users\\jdoe\\settlement.pdf")],
        },
        b"0 0 1 1 re f".to_vec(),
    );
    pdf.page(
        Page::new()
            .xobject("Fm", form)
            .content(b"/Fm Do".to_vec())
            .size(100, 100),
    );
    let doc = pdf.finish().0;
    let (bytes, stripped) = saved(&doc);
    assert!(!contains(&bytes, b"settlement"));
    assert_eq!(count(&stripped, HiddenContent::FilePath), 1);
}

/// A resource keeps only the keys its kind defines; one the
/// specification does not define is recorded as application data.
#[test]
fn a_font_keeps_only_its_keys() {
    let (doc, _) = document(shown("Hello"), Dictionary::new(), Dictionary::new());
    let (out, stripped) = rebuilt(&doc);
    assert_eq!(count(&stripped, HiddenContent::ApplicationData), 1);
    assert!(!text_of(&out).contains("Jane Roe"));
    assert!(text_of(&out).contains("Helvetica"));
}

/// A form the page lists but never draws is not copied, and is recorded.
#[test]
fn an_undrawn_form_is_left_behind() {
    let unused = (1000, 0);
    let (mut doc, _) = document(
        shown("Hello"),
        dictionary! { "XObject" => dictionary! { "Unused" => unused } },
        Dictionary::new(),
    );
    doc.objects.insert(
        unused,
        Object::Stream(Stream::new(
            dictionary! { "Type" => "XObject", "Subtype" => "Form" },
            Content {
                operations: shown("Jane Roe"),
            }
            .encode()
            .unwrap(),
        )),
    );
    let (out, stripped) = rebuilt(&doc);
    assert!(!text_of(&out).contains("Jane Roe"));
    assert_eq!(count(&stripped, HiddenContent::Undisplayed), 1);
}

/// The content of a layer hidden by default is cut, a visible layer's
/// kept, and marked content's alternate text dropped.
#[test]
fn hidden_layers_and_alternate_text_are_cut() {
    let (visible, hidden) = ((1000, 0), (1001, 0));
    let mut operations = vec![Operation::new("BDC", vec!["OC".into(), "A".into()])];
    operations.extend(shown("shown"));
    operations.push(Operation::new("EMC", vec![]));
    operations.push(Operation::new("BDC", vec!["OC".into(), "B".into()]));
    operations.extend(shown("secret"));
    operations.push(Operation::new("EMC", vec![]));
    operations.push(Operation::new(
        "BDC",
        vec![
            "Span".into(),
            Object::Dictionary(dictionary! { "ActualText" => Object::string_literal("Jane Roe") }),
        ],
    ));
    operations.extend(shown("J. R."));
    operations.push(Operation::new("EMC", vec![]));
    let (mut built, _) = document(
        operations,
        dictionary! { "Properties" => dictionary! { "A" => visible, "B" => hidden } },
        Dictionary::new(),
    );
    for id in [visible, hidden] {
        built
            .objects
            .insert(id, Object::Dictionary(dictionary! { "Type" => "OCG" }));
    }
    built.catalog_mut().unwrap().set(
        "OCProperties",
        dictionary! {
            "OCGs" => vec![visible.into(), hidden.into()],
            "D" => dictionary! { "OFF" => vec![hidden.into()] },
        },
    );

    let (out, stripped) = rebuilt(&built);
    let content = String::from_utf8_lossy(
        &out.get_page_content(out.get_pages().into_values().next().unwrap()),
    )
    .into_owned();
    assert!(content.contains("shown") && content.contains("J. R."));
    assert!(!content.contains("secret") && !text_of(&out).contains("Jane Roe"));
    assert!(!content.contains("BDC"));
    assert_eq!(count(&stripped, HiddenContent::AlternativeText), 1);
    assert!(count(&stripped, HiddenContent::Layer) >= 2);
}

/// An operator the specification does not define is cut.
#[test]
fn an_unknown_operator_is_cut() {
    let mut operations = shown("Hello");
    operations.push(Operation::new(
        "Xx",
        vec![Object::string_literal("Jane Roe")],
    ));
    let (doc, _) = document(operations, Dictionary::new(), Dictionary::new());
    let (out, _) = rebuilt(&doc);
    assert!(!text_of(&out).contains("Jane Roe"));
}

/// A page's inherited box and rotation are set on the page itself.
#[test]
fn inherited_page_attributes_are_set_on_the_page() {
    let (mut doc, page) = document(shown("Hello"), Dictionary::new(), Dictionary::new());
    doc.get_dictionary_mut(page).unwrap().remove(b"MediaBox");
    let parent = doc
        .get_dictionary(page)
        .unwrap()
        .get(b"Parent")
        .and_then(Object::as_reference)
        .unwrap();
    let tree = doc.get_dictionary_mut(parent).unwrap();
    tree.set("MediaBox", vec![0.into(), 0.into(), 300.into(), 400.into()]);
    tree.set("Rotate", 90);
    let (out, _) = rebuilt(&doc);
    let page = page_dict(&out);
    assert!(page.get(b"MediaBox").is_ok());
    assert_eq!(page.get(b"Rotate").and_then(Object::as_i64).unwrap(), 90);
}

/// A link to a named destination goes to the copied page; a link that
/// leaves the document is recorded and left behind.
#[test]
fn an_internal_link_is_resolved_to_its_page() {
    let (mut doc, page) = document(shown("Hello"), Dictionary::new(), Dictionary::new());
    let annotations = vec![
        Object::Dictionary(dictionary! {
            "Subtype" => "Link",
            "Rect" => vec![0.into(), 0.into(), 10.into(), 10.into()],
            "Dest" => Object::from("chapter"),
            "Contents" => Object::string_literal("ask Jane Roe"),
        }),
        Object::Dictionary(dictionary! {
            "Subtype" => "Link",
            "A" => dictionary! { "S" => "URI", "URI" => Object::string_literal("mailto:jane@example.com") },
        }),
    ];
    doc.get_dictionary_mut(page)
        .unwrap()
        .set("Annots", annotations);
    doc.catalog_mut().unwrap().set(
        "Dests",
        dictionary! { "chapter" => vec![page.into(), "Fit".into()] },
    );

    let (out, stripped) = rebuilt(&doc);
    let new_page = out.get_pages().into_values().next().unwrap();
    let links = page_dict(&out)
        .get(b"Annots")
        .and_then(Object::as_array)
        .unwrap();
    assert_eq!(links.len(), 1);
    let link = out
        .get_dictionary(links[0].as_reference().unwrap())
        .unwrap();
    let dest = link.get(b"Dest").and_then(Object::as_array).unwrap();
    assert_eq!(dest[0], Object::Reference(new_page));
    assert!(!text_of(&out).contains("Jane") && !text_of(&out).contains("jane@"));
    assert_eq!(count(&stripped, HiddenContent::Link), 1);
    assert_eq!(count(&stripped, HiddenContent::Annotation), 1);
}

/// A file named under two extensions is a file of each: each name keeps
/// a copy of its own, so the one whose extension the bytes match is read
/// as it is, whichever name sorts first.
#[test]
fn a_file_named_under_two_extensions_is_copied_for_each() {
    let mut doc = blank();
    let stream = doc.add_object(Stream::new(
        dictionary! { "Type" => "EmbeddedFile" },
        b"Jane Roe".to_vec(),
    ));
    // One name reaches the stream through a reference to its reference.
    let alias = doc.add_object(Object::Reference(stream));
    let spec = |doc: &mut Document, name: &str, file: ObjectId| {
        doc.add_object(dictionary! {
            "Type" => "Filespec", "F" => Object::string_literal(name),
            "EF" => dictionary! { "F" => file },
        })
    };
    let pdf = spec(&mut doc, "report.pdf", stream);
    let txt = spec(&mut doc, "notes.txt", alias);
    doc.catalog_mut().unwrap().set(
        "Names",
        dictionary! {
            "EmbeddedFiles" => dictionary! {
                "Names" => vec![
                    Object::string_literal("a"), pdf.into(),
                    Object::string_literal("b"), txt.into(),
                ],
            },
        },
    );
    let (out, _) = rebuilt(&doc);
    let entries = Tree::read(&out).unwrap().entries;
    assert_eq!(entries.len(), 2);
    assert_ne!(entries[0].stream, entries[1].stream);
    let text = entries
        .iter()
        .find(|entry| entry.name.ends_with(".txt"))
        .unwrap();
    let bytes = out.get_object(text.stream).unwrap().as_stream().unwrap();
    assert_eq!(
        bytes
            .decompressed_content()
            .unwrap_or(bytes.content.clone()),
        b"Jane Roe"
    );
    check(&out).unwrap();
}

/// A neutral name another file already took is renamed, so no two files
/// share a name.
#[test]
fn a_neutral_name_already_taken_is_renamed() {
    let source = Document::load_mem(&with_attachments(&[
        ("attachment-1.private.pdf", b"first"),
        ("attachment-1.pdf", b"second"),
    ]))
    .unwrap();
    let (out, _) = rebuilt(&source);
    let mut names: Vec<String> = Tree::read(&out)
        .unwrap()
        .entries
        .into_iter()
        .map(|entry| entry.name)
        .collect();
    names.sort();
    assert_eq!(names, ["attachment-1.pdf", "attachment-2.pdf"]);
}

/// A string names a destination only in the `/Names` tree, so a
/// catalog `/Dests` entry of the same name is not followed; and a link
/// in a layer hidden by default is no link.
#[test]
fn links_resolve_in_their_own_namespace_and_hidden_ones_are_dropped() {
    let (mut doc, page) = document(shown("Hello"), Dictionary::new(), Dictionary::new());
    let layer = doc.add_object(dictionary! { "Type" => "OCG", "Name" => "Off" });
    let link = |dest: Object| {
        dictionary! {
            "Subtype" => "Link",
            "Rect" => vec![0.into(), 0.into(), 10.into(), 10.into()],
            "Dest" => dest,
        }
    };
    let mut hidden = link(Object::from("chapter"));
    hidden.set("OC", layer);
    doc.get_dictionary_mut(page).unwrap().set(
        "Annots",
        vec![
            Object::Dictionary(link(Object::string_literal("chapter"))),
            Object::Dictionary(hidden),
        ],
    );
    doc.catalog_mut().unwrap().set(
        "Dests",
        dictionary! { "chapter" => vec![page.into(), "Fit".into()] },
    );
    doc.catalog_mut().unwrap().set(
        "OCProperties",
        dictionary! { "OCGs" => vec![layer.into()], "D" => dictionary! { "OFF" => vec![layer.into()] } },
    );
    let (out, stripped) = rebuilt(&doc);
    assert!(page_dict(&out).get(b"Annots").is_err());
    assert!(count(&stripped, HiddenContent::Layer) >= 1);
}

/// A named destination is found in the `/Names` tree past a kid that
/// does not resolve.
#[test]
fn a_named_destination_is_found_past_a_dangling_kid() {
    let (mut doc, page) = document(shown("Hello"), Dictionary::new(), Dictionary::new());
    doc.get_dictionary_mut(page).unwrap().set(
        "Annots",
        vec![Object::Dictionary(dictionary! {
            "Subtype" => "Link",
            "Rect" => vec![0.into(), 0.into(), 10.into(), 10.into()],
            "Dest" => Object::string_literal("chapter"),
        })],
    );
    let leaf = doc.add_object(dictionary! {
        "Names" => vec![Object::string_literal("chapter"), vec![page.into(), "Fit".into()].into()],
    });
    doc.catalog_mut().unwrap().set(
        "Names",
        dictionary! { "Dests" => dictionary! { "Kids" => vec![Object::Reference((9999, 0)), leaf.into()] } },
    );

    let (out, _) = rebuilt(&doc);
    let new_page = out.get_pages().into_values().next().unwrap();
    let links = page_dict(&out)
        .get(b"Annots")
        .and_then(Object::as_array)
        .unwrap();
    let link = out
        .get_dictionary(links[0].as_reference().unwrap())
        .unwrap();
    let dest = link.get(b"Dest").and_then(Object::as_array).unwrap();
    assert_eq!(dest[0], Object::Reference(new_page));
}

/// Attached files keep their content under neutral names, and a rebuild
/// of the result renames nothing.
#[test]
fn attached_files_get_neutral_names() {
    let source = Document::load_mem(&with_attachments(&[
        ("Jane_Roe_passport.pdf", b"pdf bytes"),
        ("notes.txt", b"Jane Roe"),
    ]))
    .unwrap();
    let (out, stripped) = rebuilt(&source);
    assert_eq!(count(&stripped, HiddenContent::Properties), 2);
    let mut names: Vec<String> = Tree::read(&out)
        .unwrap()
        .entries
        .into_iter()
        .map(|entry| entry.name)
        .collect();
    names.sort();
    assert_eq!(names, ["attachment-1.pdf", "attachment-2.txt"]);
    assert!(!text_of(&out).contains("Jane_Roe"));
    check(&out).unwrap();
}

/// A name that starts like a neutral one but goes on past its number is
/// renamed like any other.
#[test]
fn a_name_past_its_neutral_number_is_renamed() {
    let source = Document::load_mem(&with_attachments(&[
        ("attachment-1.Jane Roe passport scan", b"scan"),
        ("attachment-2.pdf", b"pdf bytes"),
    ]))
    .unwrap();
    let (out, stripped) = rebuilt(&source);
    assert_eq!(count(&stripped, HiddenContent::Properties), 1);
    let names: Vec<String> = Tree::read(&out)
        .unwrap()
        .entries
        .into_iter()
        .map(|entry| entry.name)
        .collect();
    assert!(names.contains(&"attachment-2.pdf".to_owned()), "{names:?}");
    assert!(names.iter().all(|name| !name.contains("Jane")), "{names:?}");
}

/// A file specification whose `/EF` is an indirect dictionary still
/// attaches its file.
#[test]
fn an_indirect_embedded_file_dictionary_is_followed() {
    let mut source = Document::load_mem(&with_attachments(&[("notes.txt", b"Jane Roe")])).unwrap();
    let spec = source
        .objects
        .iter()
        .find(|(_, object)| object.as_dict().is_ok_and(|d| d.get(b"EF").is_ok()))
        .map(|(id, _)| *id)
        .unwrap();
    let files = source
        .get_dictionary(spec)
        .unwrap()
        .get(b"EF")
        .unwrap()
        .clone();
    let files = source.add_object(files);
    source
        .get_dictionary_mut(spec)
        .unwrap()
        .set("EF", Object::Reference(files));

    let (out, _) = rebuilt(&source);
    assert_eq!(Tree::read(&out).unwrap().entries.len(), 1);
}

/// The first page's resources in `doc`.
fn resources_of(doc: &Document) -> &Dictionary {
    let resources = page_dict(doc).get(b"Resources").unwrap();
    doc.resolve_dict(resources).unwrap()
}

/// The resource `name` under `category` on the first page of `doc`,
/// resolved.
fn resource<'d>(doc: &'d Document, category: &[u8], name: &[u8]) -> &'d Object {
    let entries = resources_of(doc).get(category).unwrap();
    doc.resolve(doc.resolve_dict(entries).unwrap().get(name).unwrap())
        .unwrap()
}

/// A Type0 font's embedded encoding CMap keeps what a CMap is.
#[test]
fn a_type0_encoding_cmap_is_kept() {
    let mut ops = vec![Operation::new("BT", vec![])];
    ops.push(Operation::new("Tf", vec!["F0".into(), 12.into()]));
    ops.push(Operation::new("ET", vec![]));
    let (mut doc, _) = document(ops, Dictionary::new(), Dictionary::new());
    let cmap = doc.add_object(Stream::new(
        dictionary! {
            "Type" => "CMap", "CMapName" => "Custom", "UseCMap" => "Identity-H",
            "CIDSystemInfo" => dictionary! {
                "Registry" => Object::string_literal("Adobe"),
                "Ordering" => Object::string_literal("Identity"),
                "Supplement" => 0,
            },
            "Secret" => Object::string_literal("Jane Roe"),
        },
        b"begincmap endcmap".to_vec(),
    ));
    let font = doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type0", "BaseFont" => "X", "Encoding" => cmap,
    });
    let page = doc.get_pages().into_values().next().unwrap();
    doc.get_dictionary_mut(page)
        .unwrap()
        .get_mut(b"Resources")
        .and_then(Object::as_dict_mut)
        .unwrap()
        .set("Font", dictionary! { "F0" => font });
    let (out, _) = rebuilt(&doc);
    let Object::Dictionary(font) = resource(&out, b"Font", b"F0") else {
        panic!("font");
    };
    let encoding = out.resolve(font.get(b"Encoding").unwrap()).unwrap();
    let encoding = &encoding.as_stream().unwrap().dict;
    assert!(encoding.get(b"CMapName").is_ok() && encoding.get(b"UseCMap").is_ok());
    assert!(encoding.get(b"Secret").is_err());
}

/// A `/ToUnicode` CMap that maps a glyph to a radical maps it to the
/// ideograph, so a viewer copies the text detection read.
#[test]
fn a_to_unicode_cmap_maps_to_readable_text() {
    let mut ops = vec![Operation::new("BT", vec![])];
    ops.push(Operation::new("Tf", vec!["F0".into(), 12.into()]));
    ops.push(Operation::new("ET", vec![]));
    let (mut doc, _) = document(ops, Dictionary::new(), Dictionary::new());
    let mut cmap = Stream::new(Dictionary::new(), to_unicode(&["<01> <2F2D>"]));
    cmap.compress().unwrap();
    let cmap = doc.add_object(cmap);
    let font = doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "X", "ToUnicode" => cmap,
    });
    set_fonts(&mut doc, dictionary! { "F0" => font });
    let (out, _) = rebuilt(&doc);
    let Object::Dictionary(font) = resource(&out, b"Font", b"F0") else {
        panic!("font");
    };
    let cmap = out.resolve(font.get(b"ToUnicode").unwrap()).unwrap();
    let data = cmap.as_stream().unwrap().decompressed_content().unwrap();
    let data = String::from_utf8(data).unwrap();
    assert!(
        data.contains("<01> <5C71>") && !data.contains("2F2D"),
        "{data}"
    );
}

/// A symbol font's `/ToUnicode` is written with the text the reader reads:
/// a symbolic font's space as a space, a code the Symbol face draws as its
/// glyph's text; a font that isn't symbolic keeps the symbol range as it is.
#[test]
fn a_symbol_fonts_text_is_written_as_read() {
    let written = |base_font: &str, flags: i64| {
        let mut pdf = DocBuilder::new();
        let map = pdf.add(Stream::new(
            Dictionary::new(),
            to_unicode(&["<20> <F020>", "<6D> <F06D>"]),
        ));
        let font = pdf.add(dictionary! {
            "Type" => "Font", "Subtype" => "TrueType", "BaseFont" => base_font,
            "FontDescriptor" => dictionary! { "Type" => "FontDescriptor", "Flags" => flags },
            "ToUnicode" => map,
        });
        pdf.page(Page::new().font("F1", font).ops(shown(" m")).size(100, 100));
        let (doc, _) = pdf.finish();
        let (out, _) = rebuilt(&doc);
        let pdf = Pdf::open(&saved(&doc).0).unwrap();
        let text = pdf.read(Text).unwrap()[0].text.clone();
        let font = out
            .objects
            .values()
            .filter_map(|object| object.as_dict().ok())
            .find(|dict| dict.has(b"ToUnicode"))
            .unwrap();
        let cmap = out.resolve(font.get(b"ToUnicode").unwrap()).unwrap();
        let cmap =
            String::from_utf8_lossy(&cmap.as_stream().unwrap().decompressed_content().unwrap())
                .into_owned();
        (text, cmap)
    };
    let (text, cmap) = written("SymbolMT", 4);
    assert!(text.contains(" \u{B5}"), "{text:?}");
    assert!(
        cmap.contains("<20> <0020>") && cmap.contains("<6D> <00B5>"),
        "{cmap}"
    );
    let (_, cmap) = written("Icons", 32);
    assert!(
        cmap.contains("<20> <F020>") && cmap.contains("<6D> <F06D>"),
        "{cmap}"
    );
}

/// A `/ToUnicode` CMap mapping one-byte codes by the `bfchar` entries
/// `entries`.
fn to_unicode(entries: &[&str]) -> Vec<u8> {
    to_unicode_cmap(&["<00> <FF>"], entries, &[])
}

/// The first page's fonts in `doc` replaced by `fonts`.
fn set_fonts(doc: &mut Document, fonts: Dictionary) {
    let page = doc.get_pages().into_values().next().unwrap();
    doc.get_dictionary_mut(page)
        .unwrap()
        .get_mut(b"Resources")
        .and_then(Object::as_dict_mut)
        .unwrap()
        .set("Font", fonts);
}

/// The font `/F1` of a rebuild of a page drawing `bytes` in `font`, with
/// the rebuilt document.
fn pinned(font: Dictionary, bytes: &[u8]) -> (Document, Dictionary) {
    let (out, _) = rebuilt(&drawing_in(font, bytes));
    let Object::Dictionary(font) = resource(&out, b"Font", b"F1").clone() else {
        panic!("font");
    };
    (out, font)
}

/// A one-page document drawing `bytes` in `font`, named `/F1`.
fn drawing_in(font: Dictionary, bytes: &[u8]) -> Document {
    let mut pdf = DocBuilder::new();
    let font = pdf.add(font);
    pdf.page(
        Page::new()
            .font("F1", font)
            .ops(shown(bytes))
            .size(200, 200),
    );
    pdf.finish().0
}

/// The glyph name a pinned `font`'s `/Differences` gives `code`.
fn pinned_name(doc: &Document, font: &Dictionary, code: usize) -> String {
    let encoding = doc.resolve_dict(font.get(b"Encoding").unwrap()).unwrap();
    let differences = doc
        .resolve_array(encoding.get(b"Differences").unwrap())
        .unwrap();
    assert_eq!(differences[0], Object::Integer(0));
    String::from_utf8_lossy(differences[code + 1].as_name().unwrap()).into_owned()
}

/// A standard font without `/Widths` is rebuilt with every code's glyph
/// and width written out, and flags under which every reader takes them.
#[test]
fn a_standard_font_is_pinned() {
    let (out, font) = pinned(
        dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
            "FontDescriptor" => dictionary! { "Type" => "FontDescriptor", "Flags" => 4 },
        },
        b"Hi",
    );
    assert_eq!(font.get(b"FirstChar").unwrap(), &Object::Integer(0));
    assert_eq!(font.get(b"LastChar").unwrap(), &Object::Integer(255));
    let widths = out.resolve_array(font.get(b"Widths").unwrap()).unwrap();
    assert_eq!(widths.len(), 256);
    assert_eq!(widths[0x41], Object::Integer(667));
    assert_eq!(widths[0x80], Object::Integer(0), "no glyph");
    assert_eq!(pinned_name(&out, &font, 0x27), "quoteright");
    assert_eq!(pinned_name(&out, &font, 0x80), ".notdef");
    let descriptor = out
        .resolve_dict(font.get(b"FontDescriptor").unwrap())
        .unwrap();
    assert_eq!(descriptor.get(b"Flags").unwrap(), &Object::Integer(32));
}

/// A standard font's own `/Widths` are kept, its encoding still pinned.
#[test]
fn a_standard_fonts_own_widths_are_kept() {
    let (out, font) = pinned(
        dictionary! {
            "Type" => "Font", "Subtype" => "TrueType", "BaseFont" => "Arial",
            "FirstChar" => 65, "LastChar" => 65, "Widths" => vec![100.into()],
        },
        b"A",
    );
    assert_eq!(font.get(b"FirstChar").unwrap(), &Object::Integer(65));
    assert_eq!(
        out.resolve_array(font.get(b"Widths").unwrap()).unwrap(),
        &[Object::Integer(100)]
    );
    assert_eq!(
        pinned_name(&out, &font, 0x27),
        "quotesingle",
        "WinAnsi, a TrueType font's default"
    );
}

/// A TrueType font with no descriptor is given one, flagged nonsymbolic,
/// as a TrueType font without one reads as symbolic and ignores its
/// `/Encoding` dictionary.
#[test]
fn a_truetype_font_without_a_descriptor_is_given_one() {
    let (out, font) = pinned(
        dictionary! { "Type" => "Font", "Subtype" => "TrueType", "BaseFont" => "Arial" },
        b"A",
    );
    let descriptor = out
        .resolve_dict(font.get(b"FontDescriptor").unwrap())
        .unwrap();
    assert_eq!(descriptor.get(b"Flags").unwrap(), &Object::Integer(32));
    assert_eq!(descriptor.get(b"FontName").unwrap(), &Object::from("Arial"));
    assert_eq!(descriptor.get(b"CapHeight").unwrap(), &Object::Real(718.0));
}

/// Fonts sharing one source descriptor each get flags of their own: a
/// pinned font's descriptor is its own, and a font that isn't pinned
/// keeps the source's flags.
#[test]
fn fonts_sharing_a_descriptor_each_get_their_own_flags() {
    let mut pdf = DocBuilder::new();
    let descriptor = pdf.add(dictionary! { "Type" => "FontDescriptor", "Flags" => 64 });
    let font = |pdf: &mut DocBuilder, subtype: &str, base: &str| {
        pdf.add(dictionary! {
            "Type" => "Font", "Subtype" => subtype, "BaseFont" => base,
            "FontDescriptor" => descriptor,
        })
    };
    let arial = font(&mut pdf, "TrueType", "Arial");
    let symbol = font(&mut pdf, "Type1", "Symbol");
    let custom = font(&mut pdf, "Type1", "Custom");
    pdf.page(
        Page::new()
            .font("F1", arial)
            .font("F2", symbol)
            .font("F3", custom)
            .content(b"BT /F1 12 Tf (a) Tj /F2 12 Tf (a) Tj /F3 12 Tf (a) Tj ET".to_vec())
            .size(200, 200),
    );
    let (out, _) = rebuilt(&pdf.finish().0);
    let flags = |name: &[u8]| {
        let Object::Dictionary(font) = resource(&out, b"Font", name) else {
            panic!("font");
        };
        let descriptor = out
            .resolve_dict(font.get(b"FontDescriptor").unwrap())
            .unwrap();
        descriptor.get(b"Flags").and_then(Object::as_i64).unwrap()
    };
    assert_eq!(flags(b"F1"), 64 | 32, "Arial: nonsymbolic");
    assert_eq!(flags(b"F2"), 64 | 4, "Symbol: symbolic");
    assert_eq!(flags(b"F3"), 64, "not pinned: as the source");
}

/// Content is charged as it is read, not only as it is written: a glyph
/// procedure every glyph of a Type3 font shares, drawing nothing kept, is
/// charged once for each glyph that reads it.
#[test]
fn content_read_is_charged_each_time() {
    let mut doc = Document::with_version("1.7");
    let proc = doc.add_object(Stream::new(Dictionary::new(), b"xyz\n".repeat(2_500)));
    let font = |doc: &mut Document, glyphs: usize| {
        let procs: Dictionary = (0..glyphs)
            .map(|glyph| (format!("g{glyph}"), Object::Reference(proc)))
            .collect();
        doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type3",
            "FontMatrix" => vec![0.001.into(), 0.into(), 0.into(), 0.001.into(), 0.into(), 0.into()],
            "FontBBox" => vec![0.into(), 0.into(), 1000.into(), 1000.into()],
            "CharProcs" => procs, "Resources" => Dictionary::new(),
        })
    };
    let (one, many) = (font(&mut doc, 1), font(&mut doc, 40));
    let used = |font: ObjectId| {
        let mut builder = Builder::new(&doc);
        builder
            .copy(
                Kind::Font,
                &Object::Reference(font),
                At::top(&Scope::default()),
            )
            .unwrap();
        MAX_COPIED_BYTES - builder.copy_budget
    };
    assert!(used(many) - used(one) >= 39 * 10_000);
}

/// What pinning writes is charged to the copy budget: a budget that copies
/// a font that isn't pinned refuses the same font naming a standard face.
#[test]
fn pinning_is_charged() {
    let mut doc = Document::with_version("1.7");
    let custom = doc.add_object(type1("Custom1"));
    let helvetica = doc.add_object(type1("Helvetica"));
    let used = |doc: &Document, font: ObjectId, budget: usize| {
        let mut builder = Builder::new(doc);
        builder.copy_budget = budget;
        builder
            .copy(
                Kind::Font,
                &Object::Reference(font),
                At::top(&Scope::default()),
            )
            .map(|_| budget - builder.copy_budget)
    };
    let budget = used(&doc, custom, MAX_COPIED_BYTES).unwrap() + 64;
    assert!(used(&doc, custom, budget).is_ok());
    let error = used(&doc, helvetica, budget).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::ResourceLimit);
    assert!(used(&doc, helvetica, MAX_COPIED_BYTES).is_ok());
}

/// A font that names no standard face and embeds no program is copied
/// as it is.
#[test]
fn other_fonts_are_not_pinned() {
    let (_, custom) = pinned(
        dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Custom" },
        b"A",
    );
    assert!(custom.get(b"Encoding").is_err() && custom.get(b"Widths").is_err());
}

/// The rebuild of a page drawing `A` in a TrueType font embedding a
/// program whose `A` is 600 thousandths of an em wide, with the font
/// entries `extra`: the rebuilt font and document.
fn embedded_pinned(extra: Dictionary) -> (Document, Dictionary) {
    let mut pdf = DocBuilder::new();
    let program = Sfnt::new(1000, &[(".notdef", 300), ("A", 600)]).cmap(3, 1, 0x41, &[1]);
    let mut font = embedding(
        pdf.doc(),
        "TrueType",
        FontFile::TrueType,
        program.build(),
        32,
    );
    font.extend(&extra);
    let font = pdf.add(font);
    pdf.page(Page::new().font("F1", font).ops(shown("A")).size(200, 200));
    let (out, _) = rebuilt(&pdf.finish().0);
    let Object::Dictionary(font) = resource(&out, b"Font", b"F1").clone() else {
        panic!("font");
    };
    (out, font)
}

/// A font embedding its program without `/Widths` is rebuilt with the
/// program's widths for every code, its encoding as it was.
#[test]
fn an_embedded_fonts_program_widths_are_pinned() {
    let (out, font) = embedded_pinned(Dictionary::new());
    assert_eq!(font.get(b"FirstChar").unwrap(), &Object::Integer(0));
    assert_eq!(font.get(b"LastChar").unwrap(), &Object::Integer(255));
    let widths = out.resolve_array(font.get(b"Widths").unwrap()).unwrap();
    assert_eq!(widths.len(), 256);
    assert_eq!(widths[0x41], Object::Integer(600));
    assert_eq!(widths[0x42], Object::Integer(300), ".notdef");
    assert!(font.get(b"Encoding").is_err());
}

/// A Type1 font whose encoding leaves codes to its program's built-in one
/// is rebuilt naming every code's glyph as the program does, so its text
/// reads without the reader opening the program; a code the program maps
/// no glyph to draws nothing.
#[test]
fn a_programs_built_in_encoding_is_pinned() {
    let mut pdf = DocBuilder::new();
    let program = Type1::new(&[(".notdef", 250), ("J", 500), ("a", 500)])
        .encoding(&[(1, "J"), (2, "a")])
        .build();
    let font = embedding(pdf.doc(), "Type1", FontFile::Type1, program, 4);
    let font = pdf.add(font);
    pdf.page(
        Page::new()
            .font("F1", font)
            .ops(shown(b"\x01\x02\x03".to_vec()))
            .size(200, 200),
    );
    let (bytes, _) = saved(&pdf.finish().0);
    let out = Pdf::open(&bytes).unwrap();
    let Object::Dictionary(font) = resource(out.doc(), b"Font", b"F1").clone() else {
        panic!("font");
    };
    assert_eq!(pinned_name(out.doc(), &font, 1), "J");
    assert_eq!(pinned_name(out.doc(), &font, 3), ".notdef");
    let widths = out
        .doc()
        .resolve_array(font.get(b"Widths").unwrap())
        .unwrap();
    assert_eq!(widths[1], Object::Integer(500));
    assert_eq!(out.read(Text).unwrap()[0].text, "Ja");
}

/// A program is charged to the copy budget for each font that reads it:
/// fonts sharing one program each decode it. Two programs giving the same
/// widths, one padded, cost each further font exactly the padding more.
#[test]
fn a_shared_program_is_charged_for_each_font() {
    let extra = |padding: usize| {
        let mut doc = Document::with_version("1.7");
        let mut program = Sfnt::new(1000, &[(".notdef", 300), ("A", 600)])
            .cmap(3, 1, 0x41, &[1])
            .build();
        program.resize(program.len() + padding, 0);
        let font = embedding(&mut doc, "TrueType", FontFile::TrueType, program, 32);
        let fonts: Vec<ObjectId> = (0..3).map(|_| doc.add_object(font.clone())).collect();
        let used = |fonts: &[ObjectId]| {
            let mut builder = Builder::new(&doc);
            for &font in fonts {
                builder
                    .copy(
                        Kind::Font,
                        &Object::Reference(font),
                        At::top(&Scope::default()),
                    )
                    .unwrap();
            }
            MAX_COPIED_BYTES - builder.copy_budget
        };
        used(&fonts) - used(&fonts[..1])
    };
    assert_eq!(extra(10_000) - extra(0), 2 * 10_000);
}

/// An embedded font's own `/Widths` are kept.
#[test]
fn an_embedded_fonts_own_widths_are_kept() {
    let (out, font) = embedded_pinned(dictionary! {
        "FirstChar" => 65, "LastChar" => 65, "Widths" => vec![100.into()],
    });
    assert_eq!(font.get(b"FirstChar").unwrap(), &Object::Integer(65));
    assert_eq!(
        out.resolve_array(font.get(b"Widths").unwrap()).unwrap(),
        &[Object::Integer(100)]
    );
}

/// An embedded font's own `/Widths` are kept when its `/FirstChar` is held
/// as an object of its own.
#[test]
fn an_indirect_first_char_keeps_the_fonts_own_widths() {
    let mut pdf = DocBuilder::new();
    let first = pdf.add(Object::Integer(65));
    let program = Sfnt::new(1000, &[(".notdef", 300), ("A", 600)]).cmap(3, 1, 0x41, &[1]);
    let mut font = embedding(
        pdf.doc(),
        "TrueType",
        FontFile::TrueType,
        program.build(),
        32,
    );
    font.extend(
        &dictionary! { "FirstChar" => first, "LastChar" => 65, "Widths" => vec![900.into()] },
    );
    let font = pdf.add(font);
    pdf.page(Page::new().font("F1", font).ops(shown("A")).size(200, 200));
    let (out, _) = rebuilt(&pdf.finish().0);
    let Object::Dictionary(font) = resource(&out, b"Font", b"F1").clone() else {
        panic!("font");
    };
    assert_eq!(
        out.resolve_array(font.get(b"Widths").unwrap()).unwrap(),
        &[Object::Integer(900)]
    );
}

/// An Indexed colour space's palette given as a string is kept, as one given
/// as a stream is.
#[test]
fn an_indexed_palette_string_is_kept() {
    let palette = Object::string_literal(vec![0, 0, 0, 255, 255, 255]);
    let space = Object::Array(vec![
        "Indexed".into(),
        "DeviceRGB".into(),
        1.into(),
        palette.clone(),
    ]);
    let doc = Document::with_version("1.7");
    let mut builder = Builder::new(&doc);
    let copy = builder
        .copy(Kind::ColorSpace, &space, At::top(&Scope::default()))
        .unwrap()
        .expect("the colour space is kept");
    assert_eq!(copy.as_array().unwrap()[3], palette);
}

/// Pinning leaves the text a font reads as: the names it writes are the
/// glyphs the font's encoding already selected.
#[test]
fn a_pinned_font_reads_as_before() {
    let mut font = type1("Helvetica");
    font.set("Encoding", "WinAnsiEncoding");
    let doc = drawing_in(font, b"it's caf\xe9");
    let mut source = Vec::new();
    doc.clone().save_to(&mut source).unwrap();
    let before = Pdf::open(&source).unwrap().read(Text).unwrap();
    let (rebuilt, _) = saved(&doc);
    let after = Pdf::open(&rebuilt).unwrap().read(Text).unwrap();
    assert_eq!(after[0].text, before[0].text);
    assert_eq!(after[0].text.trim(), "it's café");
}

/// A simple font's text reads through its `/ToUnicode`, as a viewer reads
/// it, ahead of the glyph names its encoding gives (9.10.2), so it is
/// detected and deleted.
#[test]
fn a_simple_font_reads_through_its_to_unicode() {
    let ops = vec![
        Operation::new("BT", vec![]),
        Operation::new("Tf", vec!["F1".into(), 12.into()]),
        Operation::new("Tj", vec![Object::string_literal(vec![1, 2, 3, 4])]),
        Operation::new("ET", vec![]),
    ];
    let (mut doc, _) = document(ops, Dictionary::new(), Dictionary::new());
    let cmap = doc.add_object(Stream::new(
        Dictionary::new(),
        to_unicode(&["<01> <004A>", "<02> <0061>", "<03> <006E>", "<04> <0065>"]),
    ));
    let font = doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "X", "ToUnicode" => cmap,
        "Encoding" => dictionary! {
            "Differences" => vec![1.into(), "g1".into(), "g2".into(), "g3".into(), "g4".into()],
        },
    });
    set_fonts(&mut doc, dictionary! { "F1" => font });

    let (rebuilt, _) = saved(&doc);
    let pdf = Pdf::open(&rebuilt).unwrap();
    let blocks = pdf.read(Text).unwrap();
    assert_eq!(blocks[0].text.trim(), "Jane");

    let redacted = deleted(&pdf, &blocks, &[Detection::new(1, 0, 4)]);
    assert!(!redacted.read(Text).unwrap()[0].text.contains("Jane"));
}

/// A ligature some of whose characters are detected is deleted whole, so
/// its code is scrubbed from the `/ToUnicode` as no surviving text draws
/// it.
#[test]
fn a_partly_detected_ligature_is_scrubbed() {
    let ops = vec![
        Operation::new("BT", vec![]),
        Operation::new("Tf", vec!["F1".into(), 12.into()]),
        Operation::new("Tj", vec![Object::string_literal(vec![1, 2])]),
        Operation::new("ET", vec![]),
    ];
    let (mut doc, _) = document(ops, Dictionary::new(), Dictionary::new());
    let cmap = doc.add_object(Stream::new(
        Dictionary::new(),
        to_unicode(&["<01> <00660069>", "<02> <0078>"]),
    ));
    let font = doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica", "ToUnicode" => cmap,
    });
    set_fonts(&mut doc, dictionary! { "F1" => font });

    let (rebuilt, _) = saved(&doc);
    let pdf = Pdf::open(&rebuilt).unwrap();
    let blocks = pdf.read(Text).unwrap();
    assert_eq!(blocks[0].text.trim(), "fix");

    let redacted = deleted(&pdf, &blocks, &[Detection::new(1, 1, 2)]);
    let to_unicode: String = redacted
        .doc()
        .objects
        .values()
        .filter_map(|object| object.as_stream().ok())
        .filter_map(|stream| stream.decompressed_content().ok())
        .map(|content| String::from_utf8_lossy(&content).into_owned())
        .filter(|content| content.contains("beginbfchar"))
        .collect();
    assert!(
        !to_unicode.contains("00660069"),
        "the ligature's code survived"
    );
    assert!(to_unicode.contains("0078"));
}

/// A glyph in a form two pages draw, deleted for one page's detection, is
/// gone from both, so its code is scrubbed though the other page's text
/// read it too.
#[test]
fn a_glyph_deleted_from_a_shared_form_is_scrubbed() {
    let mut pdf = DocBuilder::new();
    let cmap = pdf.stream(
        Dictionary::new(),
        to_unicode(&["<01> <00660069>", "<02> <0078>"]),
    );
    let font = pdf.add(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica", "ToUnicode" => cmap,
    });
    let form = pdf.form(
        dictionary! { "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } } },
        b"BT /F1 12 Tf (\x01\x02) Tj ET".to_vec(),
    );
    for _ in 0..2 {
        pdf.page(
            Page::new()
                .xobject("Fm", form)
                .content(b"/Fm Do".to_vec())
                .size(100, 100),
        );
    }
    let (rebuilt, _) = saved(&pdf.finish().0);
    let pdf = Pdf::open(&rebuilt).unwrap();
    let blocks = pdf.read(Text).unwrap();
    assert_eq!(blocks[1].text.trim(), "fix");

    let redacted = deleted(&pdf, &blocks, &[Detection::new(1, 0, 2)]);
    let to_unicode: String = redacted
        .doc()
        .objects
        .values()
        .filter_map(|object| object.as_stream().ok())
        .filter_map(|stream| stream.decompressed_content().ok())
        .map(|content| String::from_utf8_lossy(&content).into_owned())
        .filter(|content| content.contains("beginbfchar"))
        .collect();
    assert!(
        !to_unicode.contains("00660069"),
        "the shared glyph's code survived"
    );
    assert!(to_unicode.contains("0078"));
}

/// A `/ToUnicode` lopdf cannot read is dropped, so a viewer reads no text
/// detection did not.
#[test]
fn an_unreadable_to_unicode_is_dropped() {
    let (mut doc, _) = document(shown("Hello"), Dictionary::new(), Dictionary::new());
    let cmap = doc.add_object(Stream::new(
        Dictionary::new(),
        b"1 beginbfchar <48> <004A> endbfchar".to_vec(),
    ));
    let font = doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica", "ToUnicode" => cmap,
    });
    set_fonts(&mut doc, dictionary! { "F1" => font });
    let (out, stripped) = rebuilt(&doc);
    let Object::Dictionary(font) = resource(&out, b"Font", b"F1") else {
        panic!("font");
    };
    assert!(font.get(b"ToUnicode").is_err());
    assert_eq!(count(&stripped, HiddenContent::Unreadable), 1);
}

/// A font a graphics state selects becomes a `Tf`, so its text is
/// located like any other, even after a font that could not be found.
#[test]
fn a_graphics_state_font_becomes_a_tf() {
    let ops = vec![
        Operation::new("BT", vec![]),
        Operation::new("Tf", vec!["Missing".into(), 12.into()]),
        Operation::new("gs", vec!["GS1".into()]),
        Operation::new("Tj", vec![Object::string_literal("Hello")]),
        Operation::new("ET", vec![]),
    ];
    let (mut doc, _) = document(ops, Dictionary::new(), Dictionary::new());
    let font = doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Courier",
    });
    let page = doc.get_pages().into_values().next().unwrap();
    doc.get_dictionary_mut(page)
        .unwrap()
        .get_mut(b"Resources")
        .and_then(Object::as_dict_mut)
        .unwrap()
        .set(
            "ExtGState",
            dictionary! { "GS1" => dictionary! { "Font" => vec![font.into(), 10.into()] } },
        );
    let (rebuilt, _) = saved(&doc);
    let blocks = Pdf::open(&rebuilt).unwrap().read(Text).unwrap();
    assert_eq!(blocks[0].text, "Hello");
}

/// A name the source never gave a resource selects nothing, even where
/// the rebuild gave that name a resource of its own: text shown in it
/// is cut, not shown in the font a graphics state set.
#[test]
fn a_name_the_rebuild_gave_selects_nothing_the_source_drew() {
    let ops = vec![
        Operation::new("BT", vec![]),
        Operation::new("gs", vec!["GS1".into()]),
        Operation::new("Tj", vec![Object::string_literal("Hello")]),
        Operation::new("Tf", vec!["GSFont1".into(), 12.into()]),
        Operation::new("Tj", vec![Object::string_literal("Hidden")]),
        Operation::new("ET", vec![]),
    ];
    let (mut doc, _) = document(ops, Dictionary::new(), Dictionary::new());
    let font = doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Courier",
    });
    let page = doc.get_pages().into_values().next().unwrap();
    doc.get_dictionary_mut(page)
        .unwrap()
        .get_mut(b"Resources")
        .and_then(Object::as_dict_mut)
        .unwrap()
        .set(
            "ExtGState",
            dictionary! { "GS1" => dictionary! { "Font" => vec![font.into(), 10.into()] } },
        );
    let (rebuilt, _) = saved(&doc);
    let blocks = Pdf::open(&rebuilt).unwrap().read(Text).unwrap();
    assert_eq!(blocks[0].text, "Hello");
}

/// A form shows text before its own font in its caller's: drawn where
/// that font is lost, its text is cut and reported, while the same form
/// drawn under a font that is kept keeps its text; text shown before
/// any font is selected is cut too.
#[test]
fn a_forms_text_follows_its_callers_font() {
    let mut ops = vec![
        Operation::new("Tj", vec![Object::string_literal("Before")]),
        Operation::new("BT", vec![]),
        Operation::new("Tf", vec!["F1".into(), 12.into()]),
        Operation::new("ET", vec![]),
        Operation::new("Do", vec!["Fm".into()]),
        Operation::new("BT", vec![]),
        Operation::new("Tf", vec!["Missing".into(), 12.into()]),
        Operation::new("ET", vec![]),
        Operation::new("Do", vec!["Fm".into()]),
    ];
    ops.extend(shown("Page"));
    let (mut doc, page) = document(ops, Dictionary::new(), Dictionary::new());
    let form = doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form",
            "BBox" => vec![0.into(), 0.into(), 100.into(), 100.into()],
        },
        b"BT (Inherited) Tj ET".to_vec(),
    ));
    doc.get_dictionary_mut(page)
        .unwrap()
        .get_mut(b"Resources")
        .and_then(Object::as_dict_mut)
        .unwrap()
        .set("XObject", dictionary! { "Fm" => form });
    let (bytes, stripped) = saved(&doc);
    assert!(count(&stripped, HiddenContent::Unreadable) >= 2);
    let blocks = Pdf::open(&bytes).unwrap().read(Text).unwrap();
    let text = &blocks[0].text;
    assert_eq!(text.matches("Inherited").count(), 1, "{text}");
    assert!(text.contains("Page") && !text.contains("Before"), "{text}");
}

/// A print box outside the media box is not written, and is recorded.
#[test]
fn a_print_box_dropped_is_recorded() {
    let (doc, _) = document(
        shown("Hello"),
        Dictionary::new(),
        dictionary! { "TrimBox" => vec![500.into(), 500.into(), 600.into(), 600.into()] },
    );
    let (out, stripped) = rebuilt(&doc);
    assert!(page_dict(&out).get(b"TrimBox").is_err());
    assert_eq!(count(&stripped, HiddenContent::Properties), 1);
}

/// A page transition the rebuild does not copy is recorded, so the
/// check refuses a document still carrying one.
#[test]
fn a_page_transition_is_recorded() {
    let (doc, _) = document(
        shown("Hello"),
        Dictionary::new(),
        dictionary! { "Dur" => 5, "Trans" => dictionary! { "S" => "Dissolve" } },
    );
    let (mut out, stripped) = rebuilt(&doc);
    assert_eq!(count(&stripped, HiddenContent::Properties), 2);
    let page = out.get_pages().into_values().next().unwrap();
    out.get_dictionary_mut(page)
        .unwrap()
        .set("Trans", dictionary! { "S" => "Dissolve" });
    let error = check(&out).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Sanitization);
}

/// A font a page lists but no content uses is dropped and recorded, so
/// the check refuses a document that still carries one.
#[test]
fn an_unused_font_fails_the_check() {
    let (doc, _) = document(shown("Hello"), Dictionary::new(), Dictionary::new());
    let (mut out, _) = rebuilt(&doc);
    check(&out).unwrap();
    let unused = out.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Courier",
    });
    let page = out.get_pages().into_values().next().unwrap();
    out.get_dictionary_mut(page)
        .unwrap()
        .get_mut(b"Resources")
        .and_then(Object::as_dict_mut)
        .and_then(|resources| resources.get_mut(b"Font"))
        .and_then(Object::as_dict_mut)
        .unwrap()
        .set("Unused", unused);
    let error = check(&out).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Sanitization);
}

/// A default colour space travels with the content that draws in its
/// device space; a PostScript XObject is cut as script.
#[test]
fn default_spaces_travel_and_postscript_is_cut() {
    let ops = vec![
        Operation::new("g", vec![0.5.into()]),
        Operation::new("Do", vec!["PS0".into()]),
    ];
    let (mut doc, _) = document(
        ops,
        dictionary! {
            "ColorSpace" => dictionary! {
                "DefaultGray" => vec![
                    Object::from("CalGray"),
                    Object::Dictionary(dictionary! { "WhitePoint" => vec![1.into(), 1.into(), 1.into()] }),
                ],
            },
            "XObject" => dictionary! { "PS0" => Object::Reference((1000, 0)) },
        },
        Dictionary::new(),
    );
    doc.objects.insert(
        (1000, 0),
        Object::Stream(Stream::new(
            dictionary! { "Type" => "XObject", "Subtype" => "PS" },
            b"(Jane Roe) show".to_vec(),
        )),
    );
    let (out, stripped) = rebuilt(&doc);
    assert!(matches!(
        resource(&out, b"ColorSpace", b"DefaultGray"),
        Object::Array(_)
    ));
    assert_eq!(count(&stripped, HiddenContent::Script), 1);
    assert!(!text_of(&out).contains("Jane Roe"));
}

/// A DeviceN space keeps its attributes, a soft mask its matte, a CID
/// font descriptor its per-class descriptors.
#[test]
fn nested_dictionaries_keep_their_keys() {
    let tint = Object::Dictionary(dictionary! {
        "FunctionType" => 2, "Domain" => vec![0.into(), 1.into()],
        "C0" => vec![0.into()], "C1" => vec![1.into()], "N" => 1,
    });
    let device_n = vec![
        Object::from("DeviceN"),
        vec![Object::from("Spot")].into(),
        Object::from("DeviceGray"),
        tint.clone(),
        Object::Dictionary(dictionary! {
            "Subtype" => "NChannel",
            "Colorants" => dictionary! {
                "Spot" => vec![
                    Object::from("Separation"),
                    Object::from("Spot"),
                    Object::from("DeviceGray"),
                    tint,
                ],
            },
        }),
    ];
    let ops = vec![
        Operation::new("cs", vec!["CS0".into()]),
        Operation::new("Do", vec!["Im0".into()]),
    ];
    let (mut doc, _) = document(
        ops,
        dictionary! {
            "ColorSpace" => dictionary! { "CS0" => device_n },
            "XObject" => dictionary! { "Im0" => Object::Reference((1000, 0)) },
        },
        Dictionary::new(),
    );
    let gray = |extra: Dictionary| {
        let mut dict = dictionary! {
            "Type" => "XObject", "Subtype" => "Image", "Width" => 1, "Height" => 1,
            "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8,
        };
        dict.extend(&extra);
        Object::Stream(Stream::new(dict, vec![0]))
    };
    doc.objects
        .insert((1001, 0), gray(dictionary! { "Matte" => vec![0.into()] }));
    doc.objects.insert(
        (1000, 0),
        gray(dictionary! { "SMask" => Object::Reference((1001, 0)) }),
    );
    let (out, _) = rebuilt(&doc);
    let Object::Array(space) = resource(&out, b"ColorSpace", b"CS0") else {
        panic!("colour space");
    };
    let attributes = out.resolve_dict(&space[4]).unwrap();
    assert!(attributes.get(b"Colorants").is_ok());
    let Object::Stream(image) = resource(&out, b"XObject", b"Im0") else {
        panic!("image");
    };
    let mask = out.resolve(image.dict.get(b"SMask").unwrap()).unwrap();
    assert!(mask.as_stream().unwrap().dict.get(b"Matte").is_ok());
}

/// An attachment-tree entry that embeds no file, a path to one
/// elsewhere, is recorded as a file path and not carried.
#[test]
fn an_attachment_path_is_recorded() {
    let mut pdf = DocBuilder::new();
    pdf.page(Page::new().ops(shown("Hello")).size(100, 100));
    pdf.catalog(
        "Names",
        dictionary! {
            "EmbeddedFiles" => dictionary! {
                "Names" => vec![
                    Object::string_literal("contract"),
                    Object::string_literal("C:\\Users\\jdoe\\contract.pdf"),
                ],
            },
        },
    );
    let (bytes, stripped) = saved(&pdf.finish().0);
    assert!(!contains(&bytes, b"jdoe"));
    assert_eq!(count(&stripped, HiddenContent::FilePath), 1);
}

/// A stream whose data is in another file is not carried, and is
/// recorded once, as unreadable, not as a resource nothing draws too.
#[test]
fn an_external_stream_is_cut() {
    let ops = vec![Operation::new("Do", vec!["Im0".into()])];
    let (mut doc, _) = document(
        ops,
        dictionary! { "XObject" => dictionary! { "Im0" => Object::Reference((1000, 0)) } },
        Dictionary::new(),
    );
    doc.objects.insert(
        (1000, 0),
        Object::Stream(Stream::new(
            dictionary! {
                "Type" => "XObject", "Subtype" => "Image", "Width" => 1, "Height" => 1,
                "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8,
                "F" => Object::string_literal("/home/jane/scan.raw"),
            },
            Vec::new(),
        )),
    );
    let (out, stripped) = rebuilt(&doc);
    assert!(resources_of(&out).get(b"XObject").is_err());
    assert_eq!(count(&stripped, HiddenContent::Unreadable), 1);
    assert_eq!(
        count(&stripped, HiddenContent::Undisplayed),
        1,
        "only the font nothing draws"
    );
    assert!(!text_of(&out).contains("jane"));
}

/// The header takes the catalog's later version; the page keeps its
/// print boxes clipped to its media box but not a parent's `/Group`; the
/// catalog keeps its output intents; a link keeps its border style.
#[test]
fn layout_and_colour_targets_are_kept() {
    let (mut doc, page) = document(
        shown("Hello"),
        Dictionary::new(),
        dictionary! {
            "TrimBox" => vec![10.into(), 10.into(), 300.into(), 190.into()],
            "Annots" => vec![Object::Dictionary(dictionary! {
                "Subtype" => "Link",
                "Rect" => vec![0.into(), 0.into(), 10.into(), 10.into()],
                "Dest" => vec![Object::Reference((1, 0)), "Fit".into()],
                "BS" => dictionary! { "W" => 2, "S" => "D" },
            })],
        },
    );
    let annots = page_dict_mut(&mut doc, page);
    let links = annots
        .get_mut(b"Annots")
        .and_then(Object::as_array_mut)
        .unwrap();
    let Object::Dictionary(link) = &mut links[0] else {
        panic!("link");
    };
    link.set("Dest", vec![page.into(), "Fit".into()]);
    let parent = doc
        .get_dictionary(page)
        .unwrap()
        .get(b"Parent")
        .and_then(Object::as_reference)
        .unwrap();
    doc.get_dictionary_mut(parent)
        .unwrap()
        .set("Group", dictionary! { "S" => "Transparency" });
    doc.version = "1.4".to_owned();
    let profile = doc.add_object(Stream::new(dictionary! { "N" => 3 }, b"icc".to_vec()));
    let catalog = doc.catalog_mut().unwrap();
    catalog.set("Version", "1.7");
    catalog.set(
        "OutputIntents",
        vec![Object::Dictionary(dictionary! {
            "Type" => "OutputIntent", "S" => "GTS_PDFX",
            "OutputConditionIdentifier" => Object::string_literal("FOGRA39"),
            "OutputCondition" => Object::string_literal("Jane Roe's press"),
            "Info" => Object::string_literal("proofed by jane@example.com"),
            "DestOutputProfile" => profile,
        })],
    );

    let (out, stripped) = rebuilt(&doc);
    assert_eq!(out.version, "1.7");
    let page = page_dict(&out);
    let trim = page.get(b"TrimBox").and_then(Object::as_array).unwrap();
    assert_eq!(trim[2].as_float().unwrap(), 200.0);
    assert!(page.get(b"Group").is_err());
    // The intent keeps its identifier and profile; its free-text
    // description and condition are dropped and recorded.
    let intents = out
        .resolve_array(out.catalog().unwrap().get(b"OutputIntents").unwrap())
        .unwrap();
    let intent = out.resolve_dict(&intents[0]).unwrap();
    assert_eq!(
        intent.get(b"OutputConditionIdentifier").unwrap(),
        &Object::string_literal("FOGRA39")
    );
    assert!(intent.get(b"DestOutputProfile").is_ok());
    assert!(intent.get(b"Info").is_err() && intent.get(b"OutputCondition").is_err());
    assert!(count(&stripped, HiddenContent::Properties) >= 2);
    let links = page.get(b"Annots").and_then(Object::as_array).unwrap();
    let link = out
        .get_dictionary(links[0].as_reference().unwrap())
        .unwrap();
    assert!(link.get(b"BS").is_ok());
}

fn page_dict_mut(doc: &mut Document, page: ObjectId) -> &mut Dictionary {
    doc.get_dictionary_mut(page).unwrap()
}

/// An associated file keeps its relationship to what it is attached to.
#[test]
fn a_file_keeps_its_relationship() {
    let mut source = Document::load_mem(&with_attachments(&[("invoice.xml", b"<a/>")])).unwrap();
    let spec = source
        .objects
        .iter()
        .find(|(_, object)| object.as_dict().is_ok_and(|d| d.get(b"EF").is_ok()))
        .map(|(&id, _)| id)
        .unwrap();
    source
        .get_dictionary_mut(spec)
        .unwrap()
        .set("AFRelationship", "Data");
    let (out, _) = rebuilt(&source);
    let entry = Tree::read(&out).unwrap().entries.remove(0);
    assert_eq!(
        out.resolve_dict(entry.value)
            .unwrap()
            .get(b"AFRelationship")
            .and_then(Object::as_name)
            .unwrap(),
        b"Data"
    );
}

/// Two file specifications of one file each keep their own relationship
/// and the extension of their own name; the file is copied once for
/// each extension they give it.
#[test]
fn specifications_of_one_file_stay_apart() {
    let mut source = Document::load_mem(&with_attachments(&[("invoice.xml", b"<a/>")])).unwrap();
    let (spec, stream) = source
        .objects
        .iter()
        .find_map(|(&id, object)| {
            let files = object.as_dict().ok()?.get(b"EF").ok()?.as_dict().ok()?;
            Some((id, files.get(b"F").ok()?.as_reference().ok()?))
        })
        .unwrap();
    source
        .get_dictionary_mut(spec)
        .unwrap()
        .set("AFRelationship", "Data");
    let other = source.add_object(dictionary! {
        "Type" => "Filespec", "F" => Object::string_literal("source.pdf"),
        "AFRelationship" => "Source",
        "EF" => dictionary! { "F" => stream },
    });
    // A string in `/AF` names a file elsewhere: dropped, as a path.
    source.catalog_mut().unwrap().set(
        "AF",
        vec![
            other.into(),
            Object::string_literal("C:/Users/jane/report.pdf"),
        ],
    );

    let (out, stripped) = rebuilt(&source);
    assert_eq!(count(&stripped, HiddenContent::FilePath), 1);
    let catalog = out.catalog().unwrap();
    assert_eq!(
        out.resolve_array(catalog.get(b"AF").unwrap())
            .unwrap()
            .len(),
        1
    );
    let associated = out
        .resolve_dict(&out.resolve_array(catalog.get(b"AF").unwrap()).unwrap()[0])
        .unwrap();
    let named = Tree::read(&out).unwrap().entries.remove(0);
    let relationship = |spec: &Dictionary| {
        spec.get(b"AFRelationship")
            .and_then(Object::as_name)
            .unwrap()
            .to_vec()
    };
    assert_eq!(
        relationship(out.resolve_dict(named.value).unwrap()),
        b"Data"
    );
    assert_eq!(relationship(associated), b"Source");
    assert!(named.name.ends_with(".xml"), "{}", named.name);
    assert!(associated.file_name(&out).unwrap().ends_with(".pdf"));
    let files = out
        .objects
        .values()
        .filter(|object| {
            object
                .as_stream()
                .is_ok_and(|s| s.dict.has_type(b"EmbeddedFile"))
        })
        .count();
    assert_eq!(files, 2);
}

/// A form with no resources of its own drawn many times in one scope is
/// copied once there, however many names and forms draw it, and is not
/// recorded as undisplayed: a chain of such forms each drawing the next
/// many times stays one copy per form.
#[test]
fn a_form_without_resources_is_copied_once_per_scope() {
    let mut pdf = DocBuilder::new();
    let leaf = pdf.form(Dictionary::new(), encoded(shown("Hello")));
    let middle = pdf.form(Dictionary::new(), b"/A Do /B Do /A Do".to_vec());
    let font = pdf.add(type1("Helvetica"));
    pdf.page(
        Page::new()
            .font("F1", font)
            .xobject("M", middle)
            .xobject("N", middle)
            .xobject("A", leaf)
            .xobject("B", leaf)
            .content(b"/M Do /N Do /M Do".to_vec())
            .size(100, 100),
    );
    let (doc, _) = pdf.finish();

    let (out, stripped) = rebuilt(&doc);
    let forms = out
        .objects
        .values()
        .filter(|object| {
            object
                .as_stream()
                .is_ok_and(|stream| stream.dict.is_subtype(&out, b"Form"))
        })
        .count();
    assert_eq!(forms, 2);
    assert_eq!(count(&stripped, HiddenContent::Undisplayed), 0);
}

/// A form with no resources of its own is copied for each scope it is
/// drawn in, its names resolved there.
#[test]
fn a_form_without_resources_is_copied_per_scope() {
    assert_eq!(form_fonts(None), ["Helvetica", "Courier"]);
}

/// A form whose `/Resources` is not a dictionary has none it can use, so
/// it is copied per scope too.
#[test]
fn a_form_with_unusable_resources_is_copied_per_scope() {
    assert_eq!(form_fonts(Some(Object::Null)), ["Helvetica", "Courier"]);
    assert_eq!(
        form_fonts(Some(Object::Reference((9999, 0)))),
        ["Helvetica", "Courier"]
    );
}

/// The font each of two pages' copy of one shared form selects, each page
/// naming a different font `/F1` and the form's `/Resources` `resources`.
fn form_fonts(resources: Option<Object>) -> Vec<String> {
    let mut pdf = DocBuilder::new();
    let mut dict = Dictionary::new();
    if let Some(resources) = resources {
        dict.set("Resources", resources);
    }
    let form = pdf.form(dict, encoded(shown("Hello")));
    let content = pdf.stream(Dictionary::new(), b"/Fm Do".to_vec());
    for base in ["Helvetica", "Courier"] {
        let font = pdf.add(type1(base));
        pdf.page(
            Page::new()
                .font("F1", font)
                .xobject("Fm", form)
                .set("Contents", content)
                .size(100, 100),
        );
    }
    let (doc, _) = pdf.finish();

    let (out, _) = rebuilt(&doc);
    out.get_pages()
        .into_values()
        .map(|page| {
            let resources = out
                .resolve_dict(out.get_dictionary(page).unwrap().get(b"Resources").unwrap())
                .unwrap();
            let form = out
                .resolve(
                    resources
                        .get(b"XObject")
                        .and_then(Object::as_dict)
                        .unwrap()
                        .get(b"Fm")
                        .unwrap(),
                )
                .unwrap();
            let form_resources = form.as_stream().unwrap().dict.get(b"Resources").unwrap();
            let font = out
                .resolve(
                    out.resolve_dict(form_resources)
                        .unwrap()
                        .get(b"Font")
                        .and_then(Object::as_dict)
                        .unwrap()
                        .get(b"F1")
                        .unwrap(),
                )
                .unwrap();
            String::from_utf8_lossy(
                font.as_dict()
                    .unwrap()
                    .get(b"BaseFont")
                    .and_then(Object::as_name)
                    .unwrap(),
            )
            .into_owned()
        })
        .collect()
}

/// The Type3 fonts in a rebuild of two pages sharing one Type3 font
/// whose glyph draws `/Fm`, each page naming a different form `/Fm`. The
/// font's own resources, if any, are made by `resources` from its id.
fn type3_copies(resources: impl FnOnce(&mut Document, ObjectId) -> Option<Dictionary>) -> usize {
    let out = type3_rebuilt(resources);
    out.objects
        .values()
        .filter(|object| {
            object
                .as_dict()
                .is_ok_and(|dict| dict.is_subtype(&out, b"Type3"))
        })
        .count()
}

/// The rebuild of two pages drawing one Type3 font, as [`type3_copies`]
/// builds them.
fn type3_rebuilt(
    resources: impl FnOnce(&mut Document, ObjectId) -> Option<Dictionary>,
) -> Document {
    let mut pdf = DocBuilder::new();
    let glyph = pdf.stream(Dictionary::new(), b"0 0 d0 /Fm Do".to_vec());
    let doc = pdf.doc();
    let font = doc.new_object_id();
    let mut dict = dictionary! {
        "Type" => "Font", "Subtype" => "Type3",
        "FontBBox" => vec![0.into(), 0.into(), 1.into(), 1.into()],
        "FontMatrix" => vec![1.into(), 0.into(), 0.into(), 1.into(), 0.into(), 0.into()],
        "CharProcs" => dictionary! { "a" => glyph },
        "Encoding" => dictionary! { "Differences" => vec![97.into(), Object::from("a")] },
        "FirstChar" => 97, "LastChar" => 97, "Widths" => vec![1.into()],
    };
    if let Some(resources) = resources(doc, font) {
        dict.set("Resources", resources);
    }
    doc.objects.insert(font, Object::Dictionary(dict));
    let content = pdf.stream(Dictionary::new(), b"BT /T3 1 Tf (a) Tj ET".to_vec());
    for drawing in [&b"0 0 1 1 re f"[..], b"0 0 2 2 re f"] {
        let form = pdf.form(Dictionary::new(), drawing.to_vec());
        pdf.page(
            Page::new()
                .font("T3", font)
                .xobject("Fm", form)
                .set("Contents", content)
                .size(100, 100),
        );
    }
    let (doc, _) = pdf.finish();
    rebuilt(&doc).0
}

/// A Type3 font with no resources of its own resolves its glyphs' names
/// in the scope it is drawn in, so it is copied for each.
#[test]
fn a_type3_font_without_resources_is_copied_per_scope() {
    assert_eq!(type3_copies(|_, _| None), 2);
}

/// A Type3 font with resources of its own resolves its glyphs' names in
/// them alone, so it is copied once, however many scopes draw it.
#[test]
fn a_type3_font_with_resources_is_copied_once() {
    assert_eq!(type3_copies(|_, _| Some(Dictionary::new())), 1);
}

/// A Type3 font whose glyph draws a form selecting the font again is
/// copied once, the form's copy referring back to it.
#[test]
fn a_type3_font_drawn_by_its_own_glyph_is_copied_once() {
    let out = type3_rebuilt(|doc, font| {
        let form = doc.add_object(Stream::new(
            dictionary! { "Type" => "XObject", "Subtype" => "Form" },
            b"BT /T3 1 Tf (a) Tj ET".to_vec(),
        ));
        Some(dictionary! {
            "Font" => dictionary! { "T3" => font },
            "XObject" => dictionary! { "Fm" => form },
        })
    });
    let type3: Vec<ObjectId> = out
        .objects
        .iter()
        .filter(|(_, object)| {
            object
                .as_dict()
                .is_ok_and(|dict| dict.is_subtype(&out, b"Type3"))
        })
        .map(|(&id, _)| id)
        .collect();
    assert_eq!(type3.len(), 1);
    // The glyph's form is kept, and the font it selects is that one copy.
    let font = out.get_dictionary(type3[0]).unwrap();
    let resources = out.resolve_dict(font.get(b"Resources").unwrap()).unwrap();
    let xobjects = out
        .resolve_dict(resources.get(b"XObject").unwrap())
        .unwrap();
    let (_, form) = xobjects.iter().next().expect("the glyph's form is kept");
    let form = out
        .get_object(form.as_reference().unwrap())
        .unwrap()
        .as_stream()
        .unwrap();
    let form_fonts = out
        .resolve_dict(
            out.resolve_dict(form.dict.get(b"Resources").unwrap())
                .unwrap()
                .get(b"Font")
                .unwrap(),
        )
        .unwrap();
    let selected: Vec<ObjectId> = form_fonts
        .iter()
        .filter_map(|(_, font)| font.as_reference().ok())
        .collect();
    assert_eq!(selected, type3, "the form selects the font itself");
}

/// Glyph procedures of one Type3 font whose graphics states select
/// different fonts each get a name of their own in the font's shared
/// resources, and each selects its own font.
#[test]
fn type3_glyphs_selecting_different_state_fonts_keep_them_apart() {
    let mut pdf = DocBuilder::new();
    let state = |pdf: &mut DocBuilder, base: &str| {
        let font = pdf.add(type1(base));
        dictionary! { "Font" => vec![font.into(), 1.into()] }
    };
    let (helvetica, courier) = (state(&mut pdf, "Helvetica"), state(&mut pdf, "Courier"));
    let glyph = |pdf: &mut DocBuilder, gs: &str| {
        pdf.stream(
            Dictionary::new(),
            format!("0 0 d0 /{gs} gs BT (x) Tj ET").into_bytes(),
        )
    };
    let (a, b) = (glyph(&mut pdf, "GA"), glyph(&mut pdf, "GB"));
    let font = pdf.add(dictionary! {
        "Type" => "Font", "Subtype" => "Type3",
        "FontBBox" => vec![0.into(), 0.into(), 1.into(), 1.into()],
        "FontMatrix" => vec![1.into(), 0.into(), 0.into(), 1.into(), 0.into(), 0.into()],
        "CharProcs" => dictionary! { "a" => a, "b" => b },
        "Encoding" => dictionary! {
            "Differences" => vec![97.into(), Object::from("a"), Object::from("b")],
        },
        "FirstChar" => 97, "LastChar" => 98, "Widths" => vec![1.into(), 1.into()],
        "Resources" => dictionary! {
            "ExtGState" => dictionary! { "GA" => helvetica, "GB" => courier },
        },
    });
    pdf.page(
        Page::new()
            .font("T3", font)
            .content(b"BT /T3 1 Tf (ab) Tj ET".to_vec())
            .size(100, 100),
    );
    let (doc, _) = pdf.finish();

    let (out, _) = rebuilt(&doc);
    check(&out).unwrap();
    let font = out
        .objects
        .values()
        .filter_map(|object| object.as_dict().ok())
        .find(|dict| dict.is_subtype(&out, b"Type3"))
        .unwrap();
    let fonts = out
        .resolve_dict(font.get(b"Resources").unwrap())
        .and_then(|resources| resources.get(b"Font").ok())
        .and_then(|fonts| out.resolve_dict(fonts))
        .unwrap();
    let procs = out.resolve_dict(font.get(b"CharProcs").unwrap()).unwrap();
    let selected: Vec<String> = [&b"a"[..], b"b"]
        .into_iter()
        .map(|glyph| {
            let proc = out.resolve(procs.get(glyph).unwrap()).unwrap();
            let content = proc.as_stream().unwrap().decompressed_content().unwrap();
            let content = Content::decode(&content).unwrap();
            let tf = content
                .operations
                .iter()
                .find(|op| op.operator == "Tf")
                .unwrap();
            let name = tf.operands[0].as_name().unwrap();
            let selected = out.resolve_dict(fonts.get(name).unwrap()).unwrap();
            String::from_utf8_lossy(selected.get(b"BaseFont").and_then(Object::as_name).unwrap())
                .into_owned()
        })
        .collect();
    assert_eq!(selected, ["Helvetica", "Courier"]);
}

/// A graphics state shared by two pages whose soft mask draws a form
/// with no resources of its own is copied for each page, the form's
/// names resolved in that page's resources.
#[test]
fn a_state_holding_a_borrowing_form_is_copied_per_scope() {
    let mut pdf = DocBuilder::new();
    let form = pdf.form(
        dictionary! { "BBox" => vec![0.into(), 0.into(), 100.into(), 100.into()] },
        encoded(shown("Hello")),
    );
    let state = pdf.add(dictionary! {
        "Type" => "ExtGState",
        "SMask" => dictionary! { "Type" => "Mask", "S" => "Luminosity", "G" => form },
    });
    let content = pdf.stream(Dictionary::new(), b"/S gs".to_vec());
    for base in ["Helvetica", "Courier"] {
        let font = pdf.add(type1(base));
        pdf.page(
            Page::new()
                .font("F1", font)
                .resource("ExtGState", "S", state)
                .set("Contents", content)
                .size(100, 100),
        );
    }
    let (doc, _) = pdf.finish();

    let (out, _) = rebuilt(&doc);
    let fonts: Vec<String> = out
        .get_pages()
        .into_values()
        .map(|page| {
            let resources = out
                .resolve_dict(out.get_dictionary(page).unwrap().get(b"Resources").unwrap())
                .unwrap();
            let state = resources
                .get(b"ExtGState")
                .and_then(Object::as_dict)
                .unwrap()
                .get(b"S")
                .unwrap();
            let mask = out
                .resolve_dict(out.resolve_dict(state).unwrap().get(b"SMask").unwrap())
                .unwrap();
            let form = out.resolve(mask.get(b"G").unwrap()).unwrap();
            let font = out
                .resolve_dict(form.as_stream().unwrap().dict.get(b"Resources").unwrap())
                .and_then(|resources| resources.get(b"Font").ok())
                .and_then(|fonts| out.resolve_dict(fonts))
                .and_then(|fonts| out.resolve_dict(fonts.get(b"F1").unwrap()))
                .unwrap();
            String::from_utf8_lossy(font.get(b"BaseFont").and_then(Object::as_name).unwrap())
                .into_owned()
        })
        .collect();
    assert_eq!(fonts, ["Helvetica", "Courier"]);
}

/// A form's answers are drawn into the page: a visible widget's
/// appearance (a checkbox's by its state) is flattened and its text
/// found and deleted like the page's; a hidden widget is not drawn.
#[test]
fn form_fields_are_flattened_into_the_page() {
    let (mut doc, page) = document(shown("Name:"), Dictionary::new(), Dictionary::new());
    let font = doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Courier",
    });
    let appearance = |doc: &mut Document, text: &str| {
        doc.add_object(Stream::new(
            dictionary! {
                "Type" => "XObject", "Subtype" => "Form",
                "BBox" => vec![0.into(), 0.into(), 100.into(), 20.into()],
                "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
            },
            Content {
                operations: shown(text),
            }
            .encode()
            .unwrap(),
        ))
    };
    let answer = appearance(&mut doc, "Jane Roe");
    let hidden = appearance(&mut doc, "Hidden note");
    let yes = appearance(&mut doc, "Yes");
    let off = appearance(&mut doc, "Off");
    let rect = || vec![50.into(), 100.into(), 150.into(), 120.into()];
    let widgets = vec![
        Object::Dictionary(dictionary! {
            "Subtype" => "Widget", "FT" => "Tx", "V" => Object::string_literal("Jane Roe"),
            "Rect" => rect(), "AP" => dictionary! { "N" => answer },
        }),
        Object::Dictionary(dictionary! {
            "Subtype" => "Widget", "F" => 2, "Rect" => rect(),
            "AP" => dictionary! { "N" => hidden },
        }),
        Object::Dictionary(dictionary! {
            "Subtype" => "Widget", "FT" => "Btn", "AS" => "Yes", "Rect" => rect(),
            "AP" => dictionary! { "N" => dictionary! { "Yes" => yes, "Off" => off } },
        }),
    ];
    doc.get_dictionary_mut(page).unwrap().set("Annots", widgets);

    let (rebuilt, stripped) = saved(&doc);
    assert_eq!(count(&stripped, HiddenContent::FormField), 1);
    let out = Document::load_mem(&rebuilt).unwrap();
    assert!(page_dict(&out).get(b"Annots").is_err());
    check(&out).unwrap();

    let pdf = Pdf::open(&rebuilt).unwrap();
    let blocks = pdf.read(Text).unwrap();
    let text = &blocks[0].text;
    assert!(text.contains("Jane Roe") && text.contains("Yes"), "{text}");
    assert!(!text.contains("Hidden") && !text.contains("Off"), "{text}");

    let start = text[..text.find("Jane Roe").unwrap()].chars().count();
    let redacted = deleted(&pdf, &blocks, &[Detection::new(1, start, start + 8)]);
    let redacted = redacted.doc();
    assert!(!text_of(redacted).contains("Jane Roe"));
}

/// An appearance without resources of its own resolves names in the
/// page's resources first, the form's default resources only after, and
/// a null page entry is none.
#[test]
fn a_widget_resolves_names_in_the_page_before_the_form_defaults() {
    assert_eq!(widget_font(false), b"Helvetica");
    assert_eq!(widget_font(true), b"Courier");
}

/// The font a widget appearance without resources draws `/F1` in, the
/// page's `/F1` Helvetica, or null when `null_on_page`, and the form's
/// default resources' Courier.
fn widget_font(null_on_page: bool) -> Vec<u8> {
    let (mut doc, page) = document(Vec::new(), Dictionary::new(), Dictionary::new());
    if null_on_page {
        doc.get_dictionary_mut(page)
            .unwrap()
            .get_mut(b"Resources")
            .and_then(Object::as_dict_mut)
            .unwrap()
            .set("Font", dictionary! { "F1" => Object::Null });
    }
    let courier = doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Courier",
    });
    let appearance = doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form",
            "BBox" => vec![0.into(), 0.into(), 100.into(), 20.into()],
        },
        Content {
            operations: shown("Jane"),
        }
        .encode()
        .unwrap(),
    ));
    doc.get_dictionary_mut(page).unwrap().set(
        "Annots",
        vec![Object::Dictionary(dictionary! {
            "Subtype" => "Widget", "FT" => "Tx",
            "Rect" => vec![50.into(), 100.into(), 150.into(), 120.into()],
            "AP" => dictionary! { "N" => appearance },
        })],
    );
    doc.catalog_mut().unwrap().set(
        "AcroForm",
        dictionary! { "DR" => dictionary! { "Font" => dictionary! { "F1" => courier } } },
    );
    let (out, _) = rebuilt(&doc);
    let xobjects = resources_of(&out)
        .get(b"XObject")
        .and_then(Object::as_dict)
        .unwrap();
    let (_, form) = xobjects.iter().next().unwrap();
    let form = out
        .get_object(form.as_reference().unwrap())
        .unwrap()
        .as_stream()
        .unwrap();
    let fonts = form
        .dict
        .get(b"Resources")
        .and_then(Object::as_dict)
        .and_then(|resources| resources.get(b"Font"))
        .and_then(Object::as_dict)
        .unwrap();
    let font = out
        .get_dictionary(fonts.get(b"F1").unwrap().as_reference().unwrap())
        .unwrap();
    font.get(b"BaseFont")
        .and_then(Object::as_name)
        .unwrap()
        .to_vec()
}

/// A translucent widget is composited as its annotation would be: as a
/// transparency group, with the annotation's blend mode and opacities.
#[test]
fn a_widget_keeps_its_blend_mode_and_opacity() {
    let (mut doc, page) = document(shown("Name:"), Dictionary::new(), Dictionary::new());
    let answer = doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form",
            "BBox" => vec![0.into(), 0.into(), 100.into(), 20.into()],
        },
        b"0 0 10 10 re f".to_vec(),
    ));
    doc.get_dictionary_mut(page).unwrap().set(
        "Annots",
        vec![Object::Dictionary(dictionary! {
            "Subtype" => "Widget", "BM" => "Multiply", "CA" => 0.5,
            "Rect" => vec![0.into(), 0.into(), 100.into(), 20.into()],
            "AP" => dictionary! { "N" => answer },
        })],
    );
    let (out, _) = rebuilt(&doc);
    let Object::Dictionary(state) = resource(&out, b"ExtGState", b"AnnotGS1") else {
        panic!("graphics state");
    };
    assert_eq!(state.get(b"BM").unwrap().as_name().unwrap(), b"Multiply");
    assert_eq!(state.get(b"CA").unwrap().as_float().unwrap(), 0.5);
    assert_eq!(state.get(b"ca").unwrap().as_float().unwrap(), 0.5);
    let Object::Stream(form) = resource(&out, b"XObject", b"Annot1") else {
        panic!("form");
    };
    assert!(form.dict.get(b"Group").is_ok());
    check(&out).unwrap();
}

/// A form the page draws and a widget shows is grouped only where the
/// widget is composited: the page's drawing of it stays ungrouped.
#[test]
fn a_widget_group_leaves_the_page_drawing_alone() {
    let (mut doc, page) = document(
        vec![Operation::new("Do", vec!["Fm".into()])],
        Dictionary::new(),
        Dictionary::new(),
    );
    let answer = doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form", "Resources" => dictionary! {},
            "BBox" => vec![0.into(), 0.into(), 100.into(), 20.into()],
        },
        b"0 0 10 10 re f".to_vec(),
    ));
    let dict = doc.get_dictionary_mut(page).unwrap();
    dict.get_mut(b"Resources")
        .and_then(Object::as_dict_mut)
        .unwrap()
        .set("XObject", dictionary! { "Fm" => answer });
    dict.set(
        "Annots",
        vec![Object::Dictionary(dictionary! {
            "Subtype" => "Widget", "CA" => 0.5,
            "Rect" => vec![0.into(), 0.into(), 100.into(), 20.into()],
            "AP" => dictionary! { "N" => answer },
        })],
    );
    let (out, _) = rebuilt(&doc);
    let Object::Stream(drawn) = resource(&out, b"XObject", b"Fm") else {
        panic!("form");
    };
    assert!(drawn.dict.get(b"Group").is_err());
    let Object::Stream(shown) = resource(&out, b"XObject", b"Annot1") else {
        panic!("form");
    };
    assert!(shown.dict.get(b"Group").is_ok());
    check(&out).unwrap();
}

/// An appearance with no resources of its own and no form defaults
/// resolves its font in the page's resources, so its text is kept.
#[test]
fn an_appearance_falls_back_to_the_page_resources() {
    let (mut doc, page) = document(shown("Name:"), Dictionary::new(), Dictionary::new());
    let answer = doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form",
            "BBox" => vec![0.into(), 0.into(), 100.into(), 20.into()],
        },
        Content {
            operations: shown("Jane Roe"),
        }
        .encode()
        .unwrap(),
    ));
    doc.get_dictionary_mut(page).unwrap().set(
        "Annots",
        vec![Object::Dictionary(dictionary! {
            "Subtype" => "Widget",
            "Rect" => vec![0.into(), 0.into(), 100.into(), 20.into()],
            "AP" => dictionary! { "N" => answer },
        })],
    );
    let (out, _) = rebuilt(&doc);
    let Object::Stream(form) = resource(&out, b"XObject", b"Annot1") else {
        panic!("form");
    };
    let fonts = out
        .resolve_dict(form.dict.get(b"Resources").unwrap())
        .and_then(|resources| resources.get(b"Font").ok())
        .and_then(|fonts| out.resolve_dict(fonts))
        .unwrap();
    assert!(fonts.get(b"F1").is_ok());
    check(&out).unwrap();
}

/// Text in a font that resolves nowhere, not in the appearance, the form
/// defaults nor the page, is cut and reported as unreadable, and the
/// rebuilt page still reads.
#[test]
fn text_in_a_font_found_nowhere_is_cut_and_reported() {
    let (mut doc, page) = document(shown("Name:"), Dictionary::new(), Dictionary::new());
    let answer = doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form",
            "BBox" => vec![0.into(), 0.into(), 100.into(), 20.into()],
        },
        b"BT /F9 12 Tf 0 0 Td (Jane Roe) Tj (Doe) ' ET".to_vec(),
    ));
    doc.get_dictionary_mut(page).unwrap().set(
        "Annots",
        vec![Object::Dictionary(dictionary! {
            "Subtype" => "Widget",
            "Rect" => vec![0.into(), 0.into(), 100.into(), 20.into()],
            "AP" => dictionary! { "N" => answer },
        })],
    );
    let (bytes, stripped) = saved(&doc);
    assert!(count(&stripped, HiddenContent::Unreadable) >= 1);
    let out = Document::load_mem(&bytes).unwrap();
    assert!(!text_of(&out).contains("Jane Roe"));
    let blocks = Pdf::open(&bytes).unwrap().read(Text).unwrap();
    assert_eq!(blocks[0].text.trim(), "Name:");
}

/// Widgets showing one form grouped share the one grouped copy of it.
#[test]
fn widgets_share_a_grouped_form() {
    let (mut doc, page) = document(shown("Name:"), Dictionary::new(), Dictionary::new());
    let answer = doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form",
            "BBox" => vec![0.into(), 0.into(), 100.into(), 20.into()],
        },
        b"0 0 10 10 re f".to_vec(),
    ));
    let widget = || {
        Object::Dictionary(dictionary! {
            "Subtype" => "Widget", "CA" => 0.5,
            "Rect" => vec![0.into(), 0.into(), 100.into(), 20.into()],
            "AP" => dictionary! { "N" => answer },
        })
    };
    doc.get_dictionary_mut(page)
        .unwrap()
        .set("Annots", vec![widget(), widget()]);
    let (out, _) = rebuilt(&doc);
    let shown = |name: &[u8]| match resources_of(&out)
        .get(b"XObject")
        .and_then(Object::as_dict)
        .unwrap()
        .get(name)
        .unwrap()
    {
        Object::Reference(id) => *id,
        _ => panic!("form"),
    };
    assert_eq!(shown(b"Annot1"), shown(b"Annot2"));
}

/// A widget in a layer hidden by default is not drawn into the page.
#[test]
fn a_widget_in_a_hidden_layer_stays_hidden() {
    let (mut doc, page) = document(shown("Name:"), Dictionary::new(), Dictionary::new());
    let layer = doc.add_object(dictionary! { "Type" => "OCG" });
    let answer = doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form",
            "BBox" => vec![0.into(), 0.into(), 100.into(), 20.into()],
        },
        b"0 0 10 10 re f".to_vec(),
    ));
    doc.get_dictionary_mut(page).unwrap().set(
        "Annots",
        vec![Object::Dictionary(dictionary! {
            "Subtype" => "Widget", "OC" => layer,
            "Rect" => vec![0.into(), 0.into(), 100.into(), 20.into()],
            "AP" => dictionary! { "N" => answer },
        })],
    );
    doc.catalog_mut().unwrap().set(
        "OCProperties",
        dictionary! { "OCGs" => vec![layer.into()], "D" => dictionary! { "OFF" => vec![layer.into()] } },
    );
    let (out, stripped) = rebuilt(&doc);
    assert!(resources_of(&out).get(b"XObject").is_err());
    assert_eq!(count(&stripped, HiddenContent::FormField), 0);
    assert!(count(&stripped, HiddenContent::Layer) >= 1);
}

/// A visible widget whose appearance alone is in a layer hidden by
/// default is not drawn into the page either.
#[test]
fn an_appearance_in_a_hidden_layer_stays_hidden() {
    let (mut doc, page) = document(shown("Name:"), Dictionary::new(), Dictionary::new());
    let layer = doc.add_object(dictionary! { "Type" => "OCG" });
    let answer = doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form", "OC" => layer,
            "BBox" => vec![0.into(), 0.into(), 100.into(), 20.into()],
        },
        b"0 0 10 10 re f".to_vec(),
    ));
    doc.get_dictionary_mut(page).unwrap().set(
        "Annots",
        vec![Object::Dictionary(dictionary! {
            "Subtype" => "Widget",
            "Rect" => vec![0.into(), 0.into(), 100.into(), 20.into()],
            "AP" => dictionary! { "N" => answer },
        })],
    );
    doc.catalog_mut().unwrap().set(
        "OCProperties",
        dictionary! { "OCGs" => vec![layer.into()], "D" => dictionary! { "OFF" => vec![layer.into()] } },
    );
    let (out, stripped) = rebuilt(&doc);
    assert!(resources_of(&out).get(b"XObject").is_err());
    assert!(count(&stripped, HiddenContent::Layer) >= 1);
    assert_eq!(count(&stripped, HiddenContent::FormField), 0);
    check(&out).unwrap();
}

/// A null or deleted entry among a membership's groups is no member: a
/// membership of a hidden layer and a null still hides.
#[test]
fn a_null_group_does_not_show_hidden_content() {
    let (mut doc, page) = document(
        vec![Operation::new("Do", vec!["Fm".into()])],
        Dictionary::new(),
        Dictionary::new(),
    );
    let layer = doc.add_object(dictionary! { "Type" => "OCG" });
    let form = doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form",
            "BBox" => vec![0.into(), 0.into(), 10.into(), 10.into()],
            "OC" => dictionary! {
                "Type" => "OCMD",
                "OCGs" => vec![layer.into(), Object::Null, Object::Reference((9999, 0))],
            },
        },
        b"0 0 10 10 re f".to_vec(),
    ));
    doc.get_dictionary_mut(page)
        .unwrap()
        .get_mut(b"Resources")
        .and_then(Object::as_dict_mut)
        .unwrap()
        .set("XObject", dictionary! { "Fm" => form });
    doc.catalog_mut().unwrap().set(
        "OCProperties",
        dictionary! { "OCGs" => vec![layer.into()], "D" => dictionary! { "OFF" => vec![layer.into()] } },
    );
    let (out, _) = rebuilt(&doc);
    assert!(resources_of(&out).get(b"XObject").is_err());
}

/// A page with no box anywhere is written with US Letter's, which a
/// page requires.
#[test]
fn a_page_without_a_box_gets_letter() {
    let (mut doc, page) = document(shown("Hello"), Dictionary::new(), Dictionary::new());
    doc.get_dictionary_mut(page).unwrap().remove(b"MediaBox");
    let (out, _) = rebuilt(&doc);
    let media = page_dict(&out)
        .get(b"MediaBox")
        .and_then(Object::as_array)
        .unwrap();
    let values: Vec<f32> = media.iter().map(|v| v.as_float().unwrap()).collect();
    assert_eq!(values, [0.0, 0.0, 612.0, 792.0]);
}

/// Content left unbalanced is closed in the order it was opened: a
/// marked-content sequence opened inside a text object closes inside it.
#[test]
fn unbalanced_content_is_closed_in_order() {
    let ops = vec![
        Operation::new("q", vec![]),
        Operation::new("BT", vec![]),
        Operation::new("BMC", vec!["Span".into()]),
        Operation::new("Tf", vec!["F1".into(), 12.into()]),
        Operation::new("Tj", vec![Object::string_literal("Hello")]),
    ];
    let (doc, _) = document(ops, Dictionary::new(), Dictionary::new());
    let (out, _) = rebuilt(&doc);
    let page = out.get_pages().into_values().next().unwrap();
    let operators: Vec<String> = Content::decode(&out.get_page_content(page))
        .unwrap()
        .operations
        .into_iter()
        .map(|op| op.operator)
        .collect();
    assert_eq!(operators, ["q", "BT", "BMC", "Tf", "Tj", "EMC", "ET", "Q"]);
    check(&out).unwrap();
}

/// A sequence left open at the end is closed only when its opener was
/// written: a hidden one, and any inside it, wrote none.
#[test]
fn only_written_sequences_are_closed_at_the_end() {
    let ops = vec![
        Operation::new("BMC", vec!["Span".into()]),
        Operation::new("BDC", vec!["OC".into(), "L".into()]),
        Operation::new("BMC", vec!["Span".into()]),
        Operation::new("re", vec![0.into(), 0.into(), 1.into(), 1.into()]),
    ];
    let (mut doc, _) = document(ops, Dictionary::new(), Dictionary::new());
    let layer = doc.add_object(dictionary! { "Type" => "OCG" });
    let page = doc.get_pages().into_values().next().unwrap();
    doc.get_dictionary_mut(page)
        .unwrap()
        .get_mut(b"Resources")
        .and_then(Object::as_dict_mut)
        .unwrap()
        .set("Properties", dictionary! { "L" => layer });
    doc.catalog_mut().unwrap().set(
        "OCProperties",
        dictionary! { "OCGs" => vec![layer.into()], "D" => dictionary! { "OFF" => vec![layer.into()] } },
    );
    let (out, _) = rebuilt(&doc);
    let page = out.get_pages().into_values().next().unwrap();
    let operators: Vec<String> = Content::decode(&out.get_page_content(page))
        .unwrap()
        .operations
        .into_iter()
        .map(|op| op.operator)
        .collect();
    // The path the hidden sequence builds is kept, painting nothing, so
    // a clip it would set still holds.
    assert_eq!(operators, ["BMC", "re", "EMC"]);
    check(&out).unwrap();
}

/// A page whose content is `ops`, with `/L` an optional content group
/// hidden by default, rebuilt; its content's operators and text.
fn hidden_layer_rebuild(ops: Vec<Operation>) -> (Vec<String>, String) {
    let (mut doc, _) = document(ops, Dictionary::new(), Dictionary::new());
    let layer = doc.add_object(dictionary! { "Type" => "OCG" });
    let page = doc.get_pages().into_values().next().unwrap();
    doc.get_dictionary_mut(page)
        .unwrap()
        .get_mut(b"Resources")
        .and_then(Object::as_dict_mut)
        .unwrap()
        .set("Properties", dictionary! { "L" => layer });
    doc.catalog_mut().unwrap().set(
        "OCProperties",
        dictionary! { "OCGs" => vec![layer.into()], "D" => dictionary! { "OFF" => vec![layer.into()] } },
    );
    let (out, _) = rebuilt(&doc);
    check(&out).unwrap();
    let page = out.get_pages().into_values().next().unwrap();
    let operators = Content::decode(&out.get_page_content(page))
        .unwrap()
        .operations
        .into_iter()
        .map(|op| op.operator)
        .collect();
    (operators, text_of(&out))
}

/// A hidden `"` whose spacing operand is a string is cut whole, so the
/// string does not ride along as spacing.
#[test]
fn a_malformed_hidden_quote_is_cut() {
    let (operators, text) = hidden_layer_rebuild(vec![
        Operation::new("BT", vec![]),
        Operation::new("BDC", vec!["OC".into(), "L".into()]),
        Operation::new(
            "\"",
            vec![
                Object::string_literal("Jane Roe"),
                0.into(),
                Object::string_literal("x"),
            ],
        ),
        Operation::new("EMC", vec![]),
        Operation::new("ET", vec![]),
    ]);
    assert_eq!(operators, ["BT", "ET"]);
    assert!(!text.contains("Jane Roe"));
}

/// An `ET` closing a text object in which a written sequence and, inside
/// it, a hidden one were opened closes the written one, with one `EMC`;
/// the hidden one still hides, until its own `EMC`, what follows.
#[test]
fn a_text_object_closes_only_the_written_sequences_begun_in_it() {
    let fill = || {
        [
            Operation::new("re", vec![0.into(), 0.into(), 1.into(), 1.into()]),
            Operation::new("f", vec![]),
        ]
    };
    let mut ops = vec![
        Operation::new("BT", vec![]),
        Operation::new("BMC", vec!["Span".into()]),
        Operation::new("BDC", vec!["OC".into(), "L".into()]),
        Operation::new("ET", vec![]),
    ];
    ops.extend(fill());
    ops.push(Operation::new("EMC", vec![]));
    ops.extend(fill());
    let (operators, _) = hidden_layer_rebuild(ops);
    assert_eq!(operators, ["BT", "BMC", "EMC", "ET", "re", "n", "re", "f"]);
}

/// What a hidden sequence draws is cut, but the state it sets holds
/// after it: a colour, and a clip, whose painting ends unpainted.
#[test]
fn state_set_in_a_hidden_sequence_lasts_past_it() {
    let mut ops = vec![
        Operation::new("BDC", vec!["OC".into(), "L".into()]),
        Operation::new("rg", vec![1.into(), 0.into(), 0.into()]),
        Operation::new("re", vec![0.into(), 0.into(), 50.into(), 50.into()]),
        Operation::new("W", vec![]),
        Operation::new("f", vec![]),
    ];
    ops.extend(shown("secret"));
    ops.push(Operation::new("EMC", vec![]));
    ops.push(Operation::new(
        "re",
        vec![0.into(), 0.into(), 10.into(), 10.into()],
    ));
    ops.push(Operation::new("f", vec![]));
    let (mut doc, _) = document(ops, Dictionary::new(), Dictionary::new());
    let layer = doc.add_object(dictionary! { "Type" => "OCG" });
    let page = doc.get_pages().into_values().next().unwrap();
    doc.get_dictionary_mut(page)
        .unwrap()
        .get_mut(b"Resources")
        .and_then(Object::as_dict_mut)
        .unwrap()
        .set("Properties", dictionary! { "L" => layer });
    doc.catalog_mut().unwrap().set(
        "OCProperties",
        dictionary! { "OCGs" => vec![layer.into()], "D" => dictionary! { "OFF" => vec![layer.into()] } },
    );
    let (out, _) = rebuilt(&doc);
    let page = out.get_pages().into_values().next().unwrap();
    let operators: Vec<String> = Content::decode(&out.get_page_content(page))
        .unwrap()
        .operations
        .into_iter()
        .map(|op| op.operator)
        .collect();
    assert_eq!(
        operators,
        ["rg", "re", "W", "n", "BT", "Tf", "ET", "re", "f"]
    );
    assert!(!text_of(&out).contains("secret"));
    check(&out).unwrap();
}

/// A one-page document whose page shows `text`, and its content id.
fn showing(text: &str) -> (Document, ObjectId) {
    let mut pdf = DocBuilder::with_version("1.5");
    let font = pdf.add(type1("Helvetica"));
    let content = pdf.stream(
        Dictionary::new(),
        Content {
            operations: shown(text),
        }
        .encode()
        .unwrap(),
    );
    pdf.page(
        Page::new()
            .font("F1", font)
            .set("Contents", content)
            .size(100, 100),
    );
    (pdf.finish().0, content)
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// An incremental update leaves the superseded content in the file, outside
/// the object graph; the rebuilt document never carries it.
#[test]
fn a_superseded_revision_never_ships() {
    let (mut doc, content) = showing("Jane Roe");
    let mut first = Vec::new();
    doc.save_to(&mut first).unwrap();

    let mut update =
        IncrementalDocument::create_from(first.clone(), Document::load_mem(&first).unwrap());
    update.opt_clone_object_to_new_document(content).unwrap();
    let stream = update
        .new_document
        .get_object_mut(content)
        .and_then(Object::as_stream_mut)
        .unwrap();
    stream.set_plain_content(b"BT /F1 12 Tf (public) Tj ET".to_vec());
    let mut updated = Vec::new();
    update.save_to(&mut updated).unwrap();
    assert!(
        contains(&updated, b"Jane Roe"),
        "the fixture keeps the old revision"
    );

    let rebuilt = Pdf::open(&updated).unwrap().read(Rebuild).unwrap().pdf;
    assert!(!contains(&rebuilt.bytes(), b"Jane Roe"));
    assert!(contains(&rebuilt.bytes(), b"public"));
}
