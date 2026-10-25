//! Build-time resource limits on `PatternRecognizerBuilder`:
//! - `with_union_budget` bounds the shared `RegexSet` union, and
//!   `with_variant_budget` bounds each individual variant regex — two separate
//!   budgets, because a tight per-variant cap and the (necessarily larger)
//!   builtin union cannot share one. These are the DoS defenses a caller cannot
//!   replicate by capping regex *sources*.
//! - `with_term_count_limit` / `with_term_bytes_limit` bound the shared
//!   Aho-Corasick dictionary automaton (aggregate across all dictionaries).

use elide_core::entity::builtins;
use elide_pattern::{Dictionary, PatternRecognizer, Regex, RegexBudget, Term, Variant};

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
fn tight_union_budget_rejects_oversized_set() {
    let result = PatternRecognizer::builder()
        .with_pattern(heavy_rule())
        // 1 KiB is far below what the heavy rule's compiled union needs.
        .with_union_budget(RegexBudget::nfa(1024))
        .build();

    // `PatternRecognizer` isn't `Debug`, so match rather than `expect_err`.
    let Err(err) = result else {
        panic!("a 1 KiB union budget must reject the heavy rule");
    };

    // The failure is surfaced as a validation error mentioning the compile.
    let msg = err.to_string().to_lowercase();
    assert!(
        msg.contains("regex") || msg.contains("size") || msg.contains("compil"),
        "unexpected error message: {msg}"
    );
}

#[test]
fn generous_union_budget_compiles() {
    // The same rule compiles fine under a generous budget.
    PatternRecognizer::builder()
        .with_pattern(heavy_rule())
        .with_union_budget(RegexBudget::nfa(50 * 1024 * 1024))
        .build()
        .expect("50 MiB budget compiles the heavy rule");
}

#[test]
fn tight_variant_budget_rejects_a_single_large_variant_without_breaking_the_union() {
    // One variant whose compiled NFA is large on its own.
    let big = format!("(?:{})", vec!["[a-z]{200}"; 64].join("|"));
    let rule = Regex::builder()
        .with_name("big")
        .with_labels(vec![builtins::GOVERNMENT_ID.to_ref()])
        .with_variants(vec![Variant::new(&big).expect("valid regex")])
        .build()
        .expect("rule builds");

    // A tight per-variant budget rejects the pathological single variant.
    let tight = PatternRecognizer::builder()
        .with_pattern(rule)
        .with_variant_budget(RegexBudget::nfa(4096))
        .build();
    assert!(
        tight.is_err(),
        "a tiny per-variant budget must reject the large variant"
    );

    // The same per-variant budget leaves the shipped builtins buildable: each
    // builtin variant is small, and their large *union* is bounded separately.
    PatternRecognizer::builder()
        .with_builtin_patterns()
        .with_builtin_dictionaries()
        .with_variant_budget(RegexBudget::nfa(1 << 20))
        .build()
        .expect("builtins' per-variant NFAs fit a 1 MiB per-variant budget");
}

#[test]
fn unset_budgets_are_unchanged() {
    // No budget set → the `regex` crate default applies → the heavy rule
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
    // Two dictionaries of 30 each = 60 total, over a 50 limit, proving the
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
