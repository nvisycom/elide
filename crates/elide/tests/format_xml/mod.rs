//! XML scenarios: format-specific plumbing — attribute values, comments and
//! CDATA, element-name-as-context, entity/escape round-trip, nesting, and
//! namespaces. Raw-text detection itself is the txt suite's job; these pin
//! what is unique to the XML codec.
#![allow(dead_code)]

mod attributes;
mod comments_cdata;
mod element_name_context;
mod entities_and_escapes;
mod namespaces;
mod nested_structure;
mod redaction;
