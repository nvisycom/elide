#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

mod backend;
mod component;
mod krate;

use proc_macro::TokenStream;
use syn::{DeriveInput, parse_macro_input};

/// Derive `Component`: the identity a recognizer, enricher, or operator
/// records in the audit trail.
///
/// With no attribute, the id is the deriving crate's name and version. On the
/// struct, `#[component(name = "...", version = "...")]` sets either part
/// (an operator names its algorithm and versions it by hand). On a field,
/// `#[component(name)]` takes the name from that field, and
/// `#[component(delegate)]` forwards to the component the field holds.
#[proc_macro_derive(Component, attributes(component))]
pub fn derive_component(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    component::derive(&input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Derive `Request`. The field marked `#[backend(language)]`, an
/// `Option<&LanguageTag>`, is the request's language; without one the request
/// carries none.
#[proc_macro_derive(Request, attributes(backend))]
pub fn derive_request(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    backend::derive_request(&input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Derive `Response`. The field marked `#[backend(model)]`, a
/// `ModelId`, names the model that produced the response.
#[proc_macro_derive(Response, attributes(backend))]
pub fn derive_response(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    backend::derive_response(&input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}
