//! JSON scenarios: format-specific plumbing — key-as-context, scalar handling,
//! nesting, byte-exact formatting round-trip, and `\uXXXX` escapes. Raw-text
//! detection itself is the txt suite's job; these pin what is unique to the
//! JSON codec.
#![allow(dead_code)]

mod formatting_round_trip;
mod key_context;
mod nested;
mod redaction;
mod scalar_values;
mod unicode_escapes;
