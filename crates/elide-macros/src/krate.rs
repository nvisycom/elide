//! The path derived code names `elide-core` by.

use proc_macro_crate::{FoundCrate, crate_name};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Data, DeriveInput, Error, Field, Fields, Result};

/// The path to `elide-core` from the deriving crate: its own dependency on
/// `elide-core` (under whatever name it renamed it to), else the copy the
/// `elide` facade re-exports. Inside `elide-core` itself, `elide_core` names
/// the crate through its `extern crate self` alias.
pub(crate) fn core() -> TokenStream {
    match crate_name("elide-core") {
        Ok(FoundCrate::Itself) => quote!(::elide_core),
        Ok(FoundCrate::Name(name)) => {
            let name = format_ident!("{name}");
            quote!(::#name)
        }
        Err(_) => match crate_name("elide") {
            Ok(FoundCrate::Name(name)) => {
                let name = format_ident!("{name}");
                quote!(::#name::__core)
            }
            _ => quote!(::elide_core),
        },
    }
}

/// The fields of the struct `input`; an enum or union is an error, since a
/// derived identity or contract reads a field.
pub(crate) fn fields<'a>(input: &'a DeriveInput, derive: &str) -> Result<&'a Fields> {
    match &input.data {
        Data::Struct(data) => Ok(&data.fields),
        _ => Err(Error::new_spanned(
            &input.ident,
            format!("`{derive}` can only be derived for a struct"),
        )),
    }
}

/// How derived code reads `field`, the `index`th field: by name, or by
/// position for a tuple struct.
pub(crate) fn member(field: &Field, index: usize) -> TokenStream {
    match &field.ident {
        Some(ident) => quote!(#ident),
        None => {
            let index = syn::Index::from(index);
            quote!(#index)
        }
    }
}
