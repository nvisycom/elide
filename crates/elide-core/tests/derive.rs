//! The `Component`, `Request`, and `Response` derives.

use elide_core::Component;
use elide_core::backend::{Request, Response};
use elide_core::entity::audit::ModelId;
use elide_core::primitive::{ComponentId, LanguageTag};
use hipstr::HipStr;

#[derive(Component)]
struct FromCrate;

#[derive(Component)]
#[component(name = "mask", version = "1.0.0")]
struct Fixed;

#[derive(Component)]
#[component(version = "2")]
struct NamedVersioned {
    #[component(name)]
    name: HipStr<'static>,
}

#[derive(Component)]
struct Named(#[component(name)] String);

#[derive(Component)]
struct Wrapping<C> {
    #[component(delegate)]
    inner: C,
}

#[test]
fn an_unattributed_component_is_named_and_versioned_after_its_crate() {
    assert_eq!(
        FromCrate.id(),
        ComponentId::new("elide-core", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn a_fixed_identity_is_used_as_given() {
    assert_eq!(Fixed.id(), ComponentId::new("mask", "1.0.0"));
}

#[test]
fn a_name_field_names_the_component() {
    let component = NamedVersioned {
        name: HipStr::borrowed("ner-en"),
    };
    assert_eq!(component.id(), ComponentId::new("ner-en", "2"));
    assert_eq!(
        Named("tuple".to_owned()).id(),
        ComponentId::new("tuple", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn a_delegating_component_takes_the_wrapped_identity() {
    let wrapped = Wrapping {
        inner: Wrapping { inner: Fixed },
    };
    assert_eq!(wrapped.id(), Fixed.id());
}

#[derive(Request)]
struct Hinted<'a> {
    #[backend(language)]
    language: Option<&'a LanguageTag>,
}

#[derive(Request)]
struct Unhinted;

#[test]
fn a_request_reads_its_language_from_the_marked_field() {
    let polish: LanguageTag = "pl".parse().unwrap();
    let request = Hinted {
        language: Some(&polish),
    };
    assert_eq!(request.language(), Some(&polish));
    assert_eq!(Unhinted.language(), None);
}

#[derive(Response)]
struct Answer {
    #[backend(model)]
    by: ModelId,
}

#[test]
fn a_response_names_the_model_in_the_marked_field() {
    let answer = Answer {
        by: ModelId::new("gliner"),
    };
    assert_eq!(answer.model().name, "gliner");
}
