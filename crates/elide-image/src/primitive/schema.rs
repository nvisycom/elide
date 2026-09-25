//! Per-coordinate JSON-schema names for the generic geometry primitives.
//!
//! `schemars` names a generic type by its base name alone, so `Point<u32>` and
//! `Point<f64>` would both schema-name `"Point"` and clash in one document —
//! schemars disambiguates the second to an order-dependent `"Point2"`, a name
//! that is neither stable nor meaningful to a generated client. Following the
//! way `schemars` names each concrete `NonZero` type after itself
//! (`NonZeroU32`), these macros hand-write a `JsonSchema` impl that names the
//! schema for its coordinate — `PointU32`, `BoundingBoxF64` — via
//! [`Coordinate::SCHEMA_SUFFIX`](super::Coordinate::SCHEMA_SUFFIX). Nested
//! primitives are referenced through the generator, so `BoundingBoxU32` shares
//! the `PointU32` definition rather than inlining it.

/// Hand-write a `JsonSchema` impl for a two-field geometry struct, naming the
/// schema `<Ty><suffix>` and referencing each field's schema through the
/// generator so nested primitives stay shared `$defs`. Each field carries its
/// documentation string, which becomes the property's `description` — the same
/// text `#[derive(JsonSchema)]` would lift from the field doc-comment.
macro_rules! coordinate_object_schema {
    ($ty:ident { $($field:ident : $fty:ty = $desc:literal),+ $(,)? }) => {
        impl<C: super::Coordinate + schemars::JsonSchema> schemars::JsonSchema for $ty<C> {
            fn schema_name() -> ::std::borrow::Cow<'static, str> {
                ::std::format!(concat!(stringify!($ty), "{}"), C::SCHEMA_SUFFIX).into()
            }

            fn schema_id() -> ::std::borrow::Cow<'static, str> {
                ::std::format!(
                    concat!(::std::module_path!(), "::", stringify!($ty), "{}"),
                    C::SCHEMA_SUFFIX,
                )
                .into()
            }

            fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
                $(
                    let mut $field = generator.subschema_for::<$fty>();
                    $field.insert("description".to_owned(), $desc.into());
                )+
                schemars::json_schema!({
                    "type": "object",
                    "properties": { $( stringify!($field): $field ),+ },
                    "required": [ $( stringify!($field) ),+ ],
                })
            }
        }
    };
}

/// Hand-write a `JsonSchema` impl for a transparent geometry newtype, naming
/// the schema `<Ty><suffix>` and using the inner type's schema as its body.
macro_rules! coordinate_transparent_schema {
    ($ty:ident($inner:ty)) => {
        impl<C: super::Coordinate + schemars::JsonSchema> schemars::JsonSchema for $ty<C> {
            fn schema_name() -> ::std::borrow::Cow<'static, str> {
                ::std::format!(concat!(stringify!($ty), "{}"), C::SCHEMA_SUFFIX).into()
            }

            fn schema_id() -> ::std::borrow::Cow<'static, str> {
                ::std::format!(
                    concat!(::std::module_path!(), "::", stringify!($ty), "{}"),
                    C::SCHEMA_SUFFIX,
                )
                .into()
            }

            fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
                <$inner as schemars::JsonSchema>::json_schema(generator)
            }
        }
    };
}

pub(crate) use coordinate_object_schema;
pub(crate) use coordinate_transparent_schema;
