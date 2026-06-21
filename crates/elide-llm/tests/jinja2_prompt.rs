//! End-to-end: load a [`Jinja2Prompt`] from a `.j2` template and render
//! the prompt wording against a populated payload + [`RecognizerContext`].
//!
//! Covers the minijinja code path (variable interpolation, `{% for %}`,
//! `{% if %}`, the `| join` filter, hint snippet rendering for text, and
//! nested bbox access for image). The prompt is wording-only: the response
//! shape and entity lifting are not the prompt's concern.

#![cfg(feature = "jinja2")]

use elide_core::entity::builtins;
use elide_core::modality::image::{Image, ImageData, ImageLocation};
use elide_core::modality::text::{Text, TextData, TextLocation};
use elide_core::primitive::{BoundingBox, Dimensions, Point};
use elide_core::recognition::annotation::Inclusion;
use elide_core::recognition::{RecognizerContext, Scope};
use elide_llm::prompt::{Jinja2Prompt, Prompt};

const TEXT_J2: &str = include_str!("../testdata/text.j2");
const IMAGE_J2: &str = include_str!("../testdata/image.j2");

#[test]
fn text_prompt_renders_template() {
    let prompt = Jinja2Prompt::<Text>::from_template(TEXT_J2).expect("text.j2 compiles");

    let body = "From: Alice Carter <alice.carter@acme.test>\nSubject: hello";
    let alice_start = body.find("Alice Carter").expect("alice substring");
    let alice_end = alice_start + "Alice Carter".len();

    let inclusion = Inclusion::<Text>::new(TextLocation::new(alice_start, alice_end))
        .with_name("uploader-alice")
        .with_label(builtins::PERSON_NAME.to_ref());

    let data = TextData::new(body);
    let scope = Scope::<Text>::new()
        .with_inclusions(vec![inclusion])
        .with_labels(vec!["medical".to_owned(), "gdpr-request".to_owned()]);
    let ctx = RecognizerContext::new(&scope);

    let rendered = prompt.build(&data, &ctx);
    assert!(rendered.contains(body), "source text missing: {rendered}");
    assert!(
        rendered.contains("Document labels: medical, gdpr-request"),
        "labels `| join` filter not applied: {rendered}",
    );
    assert!(
        rendered.contains("Uploader hints:"),
        "`{{% if hints %}}` branch not taken: {rendered}",
    );
    assert!(
        rendered.contains("uploader-alice (person_name): value=Alice Carter"),
        "`{{% for %}}` over hints didn't render name/kind/value: {rendered}",
    );
    assert!(
        rendered
            .contains("snippet=\"From: Alice Carter <alice.carter@acme.test>\nSubject: hello\""),
        "hint snippet not rendered: {rendered}",
    );
}

#[test]
fn image_prompt_renders_template() {
    let prompt = Jinja2Prompt::<Image>::from_template(IMAGE_J2).expect("image.j2 compiles");

    let bytes = b"\x89PNG\r\n\x1a\nfake-image-bytes".to_vec();
    let dims = Dimensions::new(640, 480);

    let inclusion = Inclusion::<Image>::new(ImageLocation::new(BoundingBox::from_origin_size(
        Point::new(10.0, 20.0),
        100.0,
        50.0,
    )))
    .with_name("uploader-face")
    .with_label(builtins::PERSON_NAME.to_ref());

    let data = ImageData::new(bytes, dims);
    let scope = Scope::<Image>::new()
        .with_inclusions(vec![inclusion])
        .with_labels(vec!["badge".to_owned()]);
    let ctx = RecognizerContext::new(&scope);

    let rendered = prompt.build(&data, &ctx);
    assert!(
        rendered.contains("Tags: badge"),
        "labels `| join` not rendered: {rendered}",
    );
    assert!(
        rendered.contains("uploader-face (person_name): bbox=[10.0,20.0,100.0x50.0]"),
        "`{{% for %}}` over image hints with nested bbox access broken: {rendered}",
    );
}
