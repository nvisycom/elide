//! `#[derive(Component)]`.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{DeriveInput, Error, LitStr, Result, Type};

use crate::krate;

/// Where the derived identity comes from.
enum Source {
    /// A name and a version, each fixed or the deriving crate's.
    Named {
        /// The name: a fixed one, a field's, or the crate's.
        name: Name,
        /// A fixed version; the crate's when absent.
        version: Option<LitStr>,
    },
    /// The component held by a field.
    Delegate { member: TokenStream, ty: Box<Type> },
}

/// Where a derived name comes from.
enum Name {
    Fixed(LitStr),
    Field(TokenStream),
    Crate,
}

pub(crate) fn derive(input: &DeriveInput) -> Result<TokenStream> {
    let core = krate::core();
    let source = source(input)?;
    let ident = &input.ident;
    let mut generics = input.generics.clone();

    let body = match source {
        Source::Delegate { member, ty } => {
            generics
                .make_where_clause()
                .predicates
                .push(syn::parse_quote!(#ty: #core::Component));
            quote!(#core::Component::id(&self.#member))
        }
        Source::Named { name, version } => {
            let name = match name {
                Name::Fixed(name) => quote!(#name),
                Name::Field(member) => quote!(::core::clone::Clone::clone(&self.#member)),
                Name::Crate => quote!(::core::env!("CARGO_PKG_NAME")),
            };
            let version = match version {
                Some(version) => quote!(#version),
                None => quote!(::core::env!("CARGO_PKG_VERSION")),
            };
            quote!(#core::primitive::ComponentId::new(#name, #version))
        }
    };

    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();
    Ok(quote! {
        #[automatically_derived]
        impl #impl_generics #core::Component for #ident #ty_generics #where_clause {
            fn id(&self) -> #core::primitive::ComponentId {
                #body
            }
        }
    })
}

/// Read the `#[component(...)]` attributes of `input` into a [`Source`].
fn source(input: &DeriveInput) -> Result<Source> {
    let mut name: Option<LitStr> = None;
    let mut version: Option<LitStr> = None;
    for attr in input
        .attrs
        .iter()
        .filter(|a| a.path().is_ident("component"))
    {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("name") {
                set_once(&mut name, meta.value()?.parse()?, &meta.path, "name")
            } else if meta.path.is_ident("version") {
                set_once(&mut version, meta.value()?.parse()?, &meta.path, "version")
            } else {
                Err(meta.error("expected `name = \"...\"` or `version = \"...\"`"))
            }
        })?;
    }

    let mut name_field: Option<TokenStream> = None;
    let mut delegate: Option<(TokenStream, Type)> = None;
    for (index, field) in krate::fields(input, "Component")?.iter().enumerate() {
        for attr in field
            .attrs
            .iter()
            .filter(|a| a.path().is_ident("component"))
        {
            attr.parse_nested_meta(|meta| {
                let member = krate::member(field, index);
                if meta.path.is_ident("name") {
                    set_once(&mut name_field, member, &meta.path, "name")
                } else if meta.path.is_ident("delegate") {
                    set_once(
                        &mut delegate,
                        (member, field.ty.clone()),
                        &meta.path,
                        "delegate",
                    )
                } else {
                    Err(meta.error("expected `name` or `delegate`"))
                }
            })?;
        }
    }

    if let Some((member, ty)) = delegate {
        if name.is_some() || version.is_some() || name_field.is_some() {
            return Err(Error::new_spanned(
                &input.ident,
                "a delegating component takes its whole identity from the field; \
                 drop `name` and `version`",
            ));
        }
        return Ok(Source::Delegate {
            member,
            ty: Box::new(ty),
        });
    }
    let name = match (name, name_field) {
        (Some(_), Some(_)) => {
            return Err(Error::new_spanned(
                &input.ident,
                "the name is either fixed on the struct or read from a field, not both",
            ));
        }
        (Some(name), None) => Name::Fixed(name),
        (None, Some(member)) => Name::Field(member),
        (None, None) => Name::Crate,
    };
    Ok(Source::Named { name, version })
}

/// Set `slot` to `value`, erroring at `path` when `what` was already given.
fn set_once<T>(slot: &mut Option<T>, value: T, path: &syn::Path, what: &str) -> Result<()> {
    if slot.is_some() {
        return Err(Error::new_spanned(path, format!("`{what}` is given twice")));
    }
    *slot = Some(value);
    Ok(())
}
