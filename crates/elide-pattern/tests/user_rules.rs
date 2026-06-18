//! End-to-end: load user-supplied patterns from the on-disk wire
//! shape (`testdata/patterns/*.toml`,
//! `testdata/dictionaries/*.{toml,csv}`) through
//! [`Regex::from_toml`], [`Dictionary::metadata_from_toml`], and
//! [`Term::from_csv`], mix them with shipped patterns, and
//! confirm a real internal-handoff document yields the custom
//! entities.

use elide_core::entity::builtins;
use elide_core::modality::text::TextData;
use elide_core::recognition::{Recognizer, RecognizerContext};
use elide_pattern::{Dictionary, PatternRecognizer, Regex, Term};

#[tokio::test]
async fn user_toml_rules_load_and_detect() {
    let employee_id = Regex::from_toml(include_str!("../testdata/patterns/employee_id.toml"))
        .expect("employee_id.toml parses");
    let product_code_pattern =
        Regex::from_toml(include_str!("../testdata/patterns/product_codes.toml"))
            .expect("product_codes.toml parses");

    let terms = Term::from_csv(include_str!("../testdata/dictionaries/product_codes.csv"))
        .expect("product_codes.csv parses");
    let product_code_dict =
        Dictionary::metadata_from_toml(include_str!("../testdata/dictionaries/product_codes.toml"))
            .expect("product_codes metadata parses")
            .with_terms(terms)
            .build()
            .expect("dictionary builds");

    // 4 rows × 3 columns; every non-empty cell becomes a term.
    assert_eq!(product_code_dict.terms.len(), 12);

    // Mix user patterns with shipped (so the input also sees email etc.).
    let recognizer = PatternRecognizer::builder()
        .with_pattern(employee_id)
        .with_pattern(product_code_pattern)
        .with_dictionary(product_code_dict)
        .with_builtin_patterns()
        .build_context_enhanced()
        .expect("recognizer builds");

    let text = include_str!("../testdata/inputs/internal.txt");
    let data = TextData::new(text.to_owned());
    let ctx = RecognizerContext::new();
    let entities = recognizer.recognize(&data, &ctx).await.expect("recognize");

    // The custom regex finds both employee numbers.
    let emp_hits: Vec<&str> = entities
        .iter()
        .filter(|e| e.label == builtins::INTERNAL_ID.to_ref())
        .map(|e| &text[e.location.start..e.location.end])
        .collect();
    assert!(
        emp_hits.contains(&"EMP-12345"),
        "expected EMP-12345 among InternalId hits, got {emp_hits:?}"
    );
    assert!(
        emp_hits.contains(&"EMP-67890"),
        "expected EMP-67890 among InternalId hits, got {emp_hits:?}"
    );

    // Both the user regex and the user dictionary should fire on
    // `WIDGET-200`: regex matches the code, dictionary matches the
    // same code as a literal term.
    assert!(
        emp_hits.contains(&"WIDGET-200"),
        "expected WIDGET-200 among InternalId hits, got {emp_hits:?}"
    );

    // Dictionary fires on the alias term `premium-widget` and the
    // canonical full name `Acme Premium Widget` (substring of "as
    // Acme Premium Widget."). Either is enough to prove the
    // dictionary layer ran.
    assert!(
        emp_hits.contains(&"premium-widget") || emp_hits.contains(&"Acme Premium Widget"),
        "expected dictionary alias/full-name hit, got {emp_hits:?}"
    );

    // Shipped email regex fires too — proves user + shipped coexist.
    assert!(
        entities
            .iter()
            .any(|e| e.label == builtins::EMAIL_ADDRESS.to_ref()
                && &text[e.location.start..e.location.end] == "counsel@example.com"),
        "expected shipped email regex to fire alongside user rules"
    );
}
