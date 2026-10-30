//! The built-in person-title walk: a person's name after a title.

mod walks;

use elide_core::primitive::ConfidenceThreshold;
use elide_pattern::walks::person_title;
use walks::names;

#[tokio::test]
async fn finds_a_person_after_a_title_and_leaves_the_title_out() {
    for (text, name) in [
        ("Dr. Hans Müller said", "Hans Müller"),
        ("Mme Claire Dubois, avocate", "Claire Dubois"),
        ("Ing. Jan Novák podepsal", "Jan Novák"),
        (
            "Herr Ludwig van Beethoven komponierte",
            "Ludwig van Beethoven",
        ),
        ("Ms. O'Neil signed.", "O'Neil"),
        ("signed by Prof. Smith.", "Smith"),
    ] {
        assert_eq!(names(person_title(), text).await, [name], "in {text:?}");
    }
}

#[tokio::test]
async fn a_title_starts_the_next_name() {
    assert_eq!(
        names(person_title(), "Mr. and Mrs. Smith arrived.").await,
        ["Smith"]
    );
    assert_eq!(
        names(person_title(), "Mr. John Smith and Mrs. Jane Smith").await,
        ["John Smith", "Jane Smith"]
    );
}

#[tokio::test]
async fn a_title_before_a_generic_word_is_not_a_person() {
    for text in [
        "the Justice Department said",
        "Dr. med. Hans",
        "Dear Sir, we",
    ] {
        assert!(names(person_title(), text).await.is_empty(), "in {text:?}");
    }
}

#[tokio::test]
async fn an_ambiguous_title_needs_context() {
    let hits = walks::found(person_title(), "Judge Alito wrote", &[]).await;
    assert_eq!(hits.len(), 1);
    assert!(!ConfidenceThreshold::BASELINE.passes(hits[0].1));
}
