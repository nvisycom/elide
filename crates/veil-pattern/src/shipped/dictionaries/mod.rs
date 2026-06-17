//! Built-in [`Dictionary`]s, embedded at compile time.
//!
//! Accessors are grouped by *scope*:
//!
//! - `world::*` — universal: brand names, ISO codes (crypto,
//!   currencies) that work in any language.
//! - `<lang>::*` — locale-specific: terms written in `<lang>`
//!   that translate when the document language changes
//!   (`en::nationalities`, future `fr::nationalites`, etc.).
//!
//! Each pairs a TOML metadata sidecar
//! (`assets/dictionaries/<scope>/<domain>/*.toml`) with a term
//! source (`*.csv` for multi-column lists, `*.txt` for
//! one-per-line), merging them via
//! [`Dictionary::metadata_from_toml`] + [`crate::Term::from_csv`] /
//! [`crate::Term::from_text`].
//!
//! [`Dictionary`]: crate::Dictionary

pub mod en;
pub mod world;

use crate::Dictionary;

/// Helper used by every per-region sub-module to define a shipped
/// dictionary accessor.
///
/// Paths are resolved with `include_str!` against the path of the
/// file that *expands* the macro, so callers in sub-modules pass
/// paths relative to themselves.
#[doc(hidden)]
#[macro_export]
macro_rules! __shipped_dictionary {
    ($(#[$meta:meta])* fn $name:ident from $meta_path:literal with csv $terms:literal) => {
        $(#[$meta])*
        #[must_use]
        pub fn $name() -> $crate::Dictionary {
            let terms = $crate::Term::from_csv(include_str!($terms))
                .expect(concat!("shipped term source `", $terms, "` parses"));
            $crate::Dictionary::metadata_from_toml(include_str!($meta_path))
                .expect(concat!("shipped metadata `", $meta_path, "` is well-formed"))
                .with_terms(terms)
                .build()
                .expect(concat!("shipped dictionary `", $meta_path, "` builds"))
        }
    };
    ($(#[$meta:meta])* fn $name:ident from $meta_path:literal with text $terms:literal) => {
        $(#[$meta])*
        #[must_use]
        pub fn $name() -> $crate::Dictionary {
            let terms = $crate::Term::from_text(include_str!($terms));
            $crate::Dictionary::metadata_from_toml(include_str!($meta_path))
                .expect(concat!("shipped metadata `", $meta_path, "` is well-formed"))
                .with_terms(terms)
                .build()
                .expect(concat!("shipped dictionary `", $meta_path, "` builds"))
        }
    };
}

/// Every built-in dictionary shipped by this crate, regardless of
/// scope.
#[must_use]
pub fn all() -> Vec<Dictionary> {
    let mut dicts = world::all();
    dicts.extend(en::all());
    dicts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_shipped_dictionary_parses() {
        for dict in all() {
            assert!(!dict.terms.is_empty(), "{} has no terms", dict.name);
        }
    }
}
