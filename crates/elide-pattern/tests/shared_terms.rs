//! End-to-end: dictionaries share one automaton, yet each matches as if it
//! scanned alone. A term two dictionaries hold matches for both, and a longer
//! term one dictionary owns, or rejects, never hides a shorter one.

use elide_core::entity::{Entity, LabelRef};
use elide_core::modality::text::{Text, TextData};
use elide_core::primitive::CountryCode;
use elide_core::recognition::{Context, Recognizer, Scope, Subject};
use elide_pattern::{Dictionary, PatternRecognizer, Term};

fn dictionary(name: &str, label: &str, terms: &[&str]) -> Dictionary {
    Dictionary::builder()
        .with_name(name)
        .with_labels(vec![LabelRef::new(label)])
        .with_terms(terms.iter().copied().map(Term::new).collect::<Vec<_>>())
        .build()
        .expect("dictionary builds")
}

async fn scan(recognizer: &PatternRecognizer, text: &str, scope: Scope) -> Vec<(String, String)> {
    let ctx = Context::new(&scope);
    let subject = Subject::<Text>::new(TextData::new(text.to_owned()));
    let entities: Vec<Entity<Text>> = recognizer
        .recognize(&subject, &ctx)
        .await
        .expect("recognize");
    entities
        .iter()
        .map(|e| {
            let range = e.location.range().expect("text range");
            (
                e.label.as_str().to_owned(),
                text[range.start..range.end].to_owned(),
            )
        })
        .collect()
}

fn hit(label: &str, value: &str) -> (String, String) {
    (label.to_owned(), value.to_owned())
}

#[tokio::test]
async fn a_term_in_two_dictionaries_matches_for_both() {
    let recognizer = PatternRecognizer::builder()
        .with_dictionary(dictionary("languages", "language", &["Polish"]))
        .with_dictionary(dictionary("nationalities", "nationality", &["Polish"]))
        .build()
        .expect("recognizer builds");

    let found = scan(&recognizer, "She is Polish.", Scope::new()).await;
    assert!(found.contains(&hit("language", "Polish")), "{found:?}");
    assert!(found.contains(&hit("nationality", "Polish")), "{found:?}");
}

#[tokio::test]
async fn a_longer_term_out_of_scope_does_not_hide_a_shorter_one() {
    let us_only = dictionary("cities", "location", &["Jersey City"]);
    let us_only = Dictionary {
        countries: vec!["US".parse::<CountryCode>().unwrap()],
        ..us_only
    };
    let recognizer = PatternRecognizer::builder()
        .with_dictionary(us_only)
        .with_dictionary(dictionary("islands", "island", &["Jersey"]))
        .build()
        .expect("recognizer builds");

    let scope = Scope::new().with_country("GB".parse::<CountryCode>().unwrap());
    let found = scan(&recognizer, "Born in Jersey City.", scope).await;
    assert_eq!(found, vec![hit("island", "Jersey")]);
}

#[tokio::test]
async fn a_longer_term_that_is_not_a_whole_word_does_not_hide_a_shorter_one() {
    let recognizer = PatternRecognizer::builder()
        .with_dictionary(dictionary("ids", "id", &["tax", "tax i"]))
        .build()
        .expect("recognizer builds");

    let found = scan(&recognizer, "the tax id", Scope::new()).await;
    assert_eq!(found, vec![hit("id", "tax")]);
}

#[tokio::test]
async fn a_dictionary_keeps_its_longest_term_at_a_position() {
    let recognizer = PatternRecognizer::builder()
        .with_dictionary(dictionary("cities", "location", &["New York", "York"]))
        .build()
        .expect("recognizer builds");

    let found = scan(&recognizer, "in New York today", Scope::new()).await;
    assert_eq!(found, vec![hit("location", "New York")]);
}

#[tokio::test]
async fn nested_phrases_keep_the_longest_from_left_to_right() {
    let recognizer = PatternRecognizer::builder()
        .with_dictionary(dictionary("phrases", "phrase", &["a", "a a", "a a a"]))
        .build()
        .expect("recognizer builds");

    let found = scan(&recognizer, "a a a a", Scope::new()).await;
    assert_eq!(found, vec![hit("phrase", "a a a"), hit("phrase", "a")]);
}
