//! The built-in institution walk: institutions named around a head word.

mod walks;

use elide_core::primitive::ConfidenceThreshold;
use elide_pattern::walks::institution;
use walks::names;

#[tokio::test]
async fn finds_an_institution_around_its_head() {
    for (text, name) in [
        (
            "the U.S. Department of Justice filed suit",
            "U.S. Department of Justice",
        ),
        (
            "appealed to the Supreme Court of Canada.",
            "Supreme Court of Canada",
        ),
        (
            "She studied at Harvard University and left.",
            "Harvard University",
        ),
        ("the Ministry of Finance said", "Ministry of Finance"),
        (
            "the Court of Appeals for the Ninth Circuit held",
            "Court of Appeals for the Ninth Circuit",
        ),
        (
            "the Department of Health and Human Services",
            "Department of Health and Human Services",
        ),
        (
            "the Department of Office Services said",
            "Department of Office Services",
        ),
    ] {
        assert_eq!(names(institution(), text).await, [name], "in {text:?}");
    }
}

#[tokio::test]
async fn a_generic_phrase_is_not_an_institution() {
    for text in [
        "the Audit Committee approved it",
        "the Board of Directors met",
        "the Company's Compensation Committee",
        "the Court held that",
        "the Finance Ministry said",
    ] {
        assert!(names(institution(), text).await.is_empty(), "in {text:?}");
    }
}

#[tokio::test]
async fn an_ambiguous_head_needs_context() {
    let hits = walks::found(institution(), "the Microsoft Office suite", &[]).await;
    assert_eq!(hits.len(), 1);
    assert!(!ConfidenceThreshold::BASELINE.passes(hits[0].1));

    let hits = walks::found(institution(), "filed with the Patent Office", &[]).await;
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].0, "Patent Office");
    assert!(ConfidenceThreshold::BASELINE.passes(hits[0].1));
}
