//! Build-time resource limits on `PatternRecognizerBuilder`:
//! - `with_size_limit` bounds the compiled automaton size of both individual
//!   variant regexes and the shared `RegexSet` union — the one DoS defense a
//!   caller cannot replicate by capping regex *sources*.
//! - `with_term_count_limit` / `with_term_bytes_limit` bound the shared
//!   Aho-Corasick dictionary automaton (aggregate across all dictionaries).

use elide_core::entity::builtins;
use elide_pattern::{Dictionary, PatternRecognizer, Regex, Term, Variant};

/// A rule whose variants union into a large automaton. Each source is small,
/// but bounded-repetition alternations expand the compiled NFA/DFA, so the
/// combined `RegexSet` blows a tight byte budget.
fn heavy_rule() -> Regex {
    let variants: Vec<Variant> = (0..8)
        .map(|i| {
            // `[a-z]{200}` × several, distinct per variant so they don't dedup.
            let src = format!(r"(?:[a-z{}]{{200}}){{4}}", char::from(b'a' + i));
            Variant::new(&src).expect("variant source is a valid regex")
        })
        .collect();
    Regex::builder()
        .with_name("heavy")
        .with_labels(vec![builtins::GOVERNMENT_ID.to_ref()])
        .with_variants(variants)
        .build()
        .expect("rule builds")
}

#[test]
fn tight_size_limit_rejects_oversized_set() {
    let result = PatternRecognizer::builder()
        .with_pattern(heavy_rule())
        // 1 KiB is far below what the heavy rule's compiled automaton needs.
        .with_size_limit(1024)
        .build();

    // `PatternRecognizer` isn't `Debug`, so match rather than `expect_err`.
    let Err(err) = result else {
        panic!("a 1 KiB automaton budget must reject the heavy rule");
    };

    // The failure is surfaced as a validation error mentioning the compile.
    let msg = err.to_string().to_lowercase();
    assert!(
        msg.contains("regex") || msg.contains("size") || msg.contains("compil"),
        "unexpected error message: {msg}"
    );
}

#[test]
fn generous_size_limit_compiles() {
    // The same rule compiles fine under a generous budget.
    PatternRecognizer::builder()
        .with_pattern(heavy_rule())
        .with_size_limit(50 * 1024 * 1024)
        .build()
        .expect("50 MiB budget compiles the heavy rule");
}

#[test]
fn unset_limit_is_unchanged() {
    // No limit set → the `regex` crate default applies → the heavy rule
    // (well under the ~10 MB default) compiles exactly as before.
    PatternRecognizer::builder()
        .with_pattern(heavy_rule())
        .build()
        .expect("unconfigured builder compiles as before");
}

/// A dictionary of `n` short terms (`term0`, `term1`, …).
fn dict_of(n: usize) -> Dictionary {
    let terms: Vec<Term> = (0..n).map(|i| Term::new(format!("term{i}"))).collect();
    Dictionary::builder()
        .with_name("d")
        .with_labels(vec![builtins::GOVERNMENT_ID.to_ref()])
        .with_terms(terms)
        .build()
        .expect("dictionary builds")
}

#[test]
fn term_count_limit_rejects_over_budget() {
    let result = PatternRecognizer::builder()
        .with_dictionary(dict_of(100))
        .with_term_count_limit(50)
        .build();

    let Err(err) = result else {
        panic!("100 terms must exceed a 50-term limit");
    };
    let msg = err.to_string().to_lowercase();
    assert!(msg.contains("term count"), "unexpected error: {msg}");
}

#[test]
fn term_count_limit_aggregates_across_dictionaries() {
    // Two dictionaries of 30 each = 60 total, over a 50 limit — proving the
    // cap is recognizer-wide, not per-dictionary.
    let result = PatternRecognizer::builder()
        .with_dictionary(dict_of(30))
        .with_dictionary(dict_of(30))
        .with_term_count_limit(50)
        .build();
    assert!(result.is_err(), "aggregate 60 terms must exceed limit 50");
}

#[test]
fn term_bytes_limit_rejects_over_budget() {
    // dict_of(100) has terms "term0".."term99": 5–6 bytes each, > 500 bytes.
    let result = PatternRecognizer::builder()
        .with_dictionary(dict_of(100))
        .with_term_bytes_limit(100)
        .build();

    let Err(err) = result else {
        panic!("term bytes must exceed a 100-byte limit");
    };
    let msg = err.to_string().to_lowercase();
    assert!(msg.contains("term bytes"), "unexpected error: {msg}");
}

#[test]
fn dictionary_limits_unset_and_generous_compile() {
    // Unset: compiles. Generous: compiles.
    PatternRecognizer::builder()
        .with_dictionary(dict_of(100))
        .build()
        .expect("unset dictionary limits compile");
    PatternRecognizer::builder()
        .with_dictionary(dict_of(100))
        .with_term_count_limit(1000)
        .with_term_bytes_limit(1_000_000)
        .build()
        .expect("generous dictionary limits compile");
}
