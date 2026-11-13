//! `#[derive(Request)]` and `#[derive(Response)]`.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{DeriveInput, Error, Result};

use crate::krate;

pub(crate) fn derive_request(input: &DeriveInput) -> Result<TokenStream> {
    let core = krate::core();
    let ident = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
    let language = marked(input, "Request", "language")?.map(|member| {
        quote! {
            fn language(&self) -> ::core::option::Option<&#core::primitive::LanguageTag> {
                self.#member
            }
        }
    });
    Ok(quote! {
        #[automatically_derived]
        impl #impl_generics #core::backend::Request for #ident #ty_generics #where_clause {
            #language
        }
    })
}

pub(crate) fn derive_response(input: &DeriveInput) -> Result<TokenStream> {
    let core = krate::core();
    let ident = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
    let Some(model) = marked(input, "Response", "model")? else {
        return Err(Error::new_spanned(
            ident,
            "mark the field naming the model that produced the response with `#[backend(model)]`",
        ));
    };
    Ok(quote! {
        #[automatically_derived]
        impl #impl_generics #core::backend::Response for #ident #ty_generics #where_clause {
            fn model(&self) -> &#core::entity::audit::ModelId {
                &self.#model
            }
        }
    })
}

/// The field of `input` marked `#[backend(<key>)]`, if any.
fn marked(input: &DeriveInput, derive: &str, key: &str) -> Result<Option<TokenStream>> {
    let mut found = None;
    for (index, field) in krate::fields(input, derive)?.iter().enumerate() {
        for attr in field.attrs.iter().filter(|a| a.path().is_ident("backend")) {
            attr.parse_nested_meta(|meta| {
                if !meta.path.is_ident(key) {
                    return Err(meta.error(format!("expected `{key}`")));
                }
                if found.is_some() {
                    return Err(meta.error(format!("only one field is `{key}`")));
                }
                found = Some(krate::member(field, index));
                Ok(())
            })?;
        }
    }
    Ok(found)
}
