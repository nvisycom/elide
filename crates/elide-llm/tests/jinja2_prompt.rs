//! End-to-end: load a [`Jinja2Prompt`] from a `.j2` template and render
//! the prompt wording against a populated payload + [`Context`].
//!
//! Covers the minijinja code path (variable interpolation, `{% if %}`, and the
//! `| join` filter) for text and image. The prompt is wording-only: the
//! response shape and entity lifting are not the prompt's concern.

#![cfg(feature = "jinja2")]

use elide_core::entity::{LabelCatalog, builtins};
use elide_core::modality::text::{Text, TextData};
use elide_core::recognition::{Context, Scope, Subject};
use elide_image::modality::Image;
use elide_llm::prompt::{Jinja2Prompt, Prompt};

const TEXT_J2: &str = include_str!("../testdata/text.j2");
const IMAGE_J2: &str = include_str!("../testdata/image.j2");

#[test]
fn text_prompt_renders_template() {
    let prompt = Jinja2Prompt::<Text>::from_template(TEXT_J2).expect("text.j2 compiles");

    let body = "From: Alice Carter <alice.carter@acme.test>\nSubject: hello";
    let scope = Scope::new()
        .with_tags(vec!["medical".to_owned(), "gdpr-request".to_owned()])
        .with_catalog(LabelCatalog::from_iter([builtins::PERSON_NAME.clone()]));
    let ctx = Context::new(&scope);
    let subject = Subject::<Text>::new(TextData::new(body));

    let rendered = prompt.build(&subject, &ctx);
    assert!(rendered.contains(body), "source text missing: {rendered}");
    assert!(
        rendered.contains("Document tags: medical, gdpr-request"),
        "tags `| join` filter not applied: {rendered}",
    );
    assert!(
        rendered.contains("Emit only: person_name"),
        "target labels not rendered: {rendered}",
    );
}

#[test]
fn image_prompt_renders_template() {
    let prompt = Jinja2Prompt::<Image>::from_template(IMAGE_J2).expect("image.j2 compiles");

    let data = elide_image::fixtures::image_data(
        elide_image::fixtures::png(4, 4),
        elide_image::modality::ImageFormat::Png,
    );
    let scope = Scope::new().with_tags(vec!["badge".to_owned()]);
    let ctx = Context::new(&scope);
    let subject = Subject::<Image>::new(data);

    let rendered = prompt.build(&subject, &ctx);
    assert!(
        rendered.contains("Tags: badge"),
        "tags `| join` not rendered: {rendered}",
    );
}
