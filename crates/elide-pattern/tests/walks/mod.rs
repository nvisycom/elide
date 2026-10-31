//! Shared helpers for the `builtin_walk_*` test suites.

use elide_core::modality::text::{Text, TextData};
use elide_core::primitive::{Confidence, LanguageTag};
use elide_core::recognition::{Context, Recognizer, Scope, Subject};
use elide_pattern::{PatternRecognizer, Walk};

/// Every name `walk` finds in `text` under the asserted `languages`, with its
/// confidence, in text order.
pub async fn found(walk: Walk, text: &str, languages: &[&str]) -> Vec<(String, Confidence)> {
    let scope = languages.iter().fold(Scope::new(), |scope, language| {
        scope.with_language(language.parse::<LanguageTag>().expect("language tag"))
    });
    found_in(walk, Subject::new(TextData::new(text.to_owned())), &scope).await
}

/// Every name `walk` finds in `subject` under `scope`, with its confidence,
/// in text order.
pub async fn found_in(
    walk: Walk,
    subject: Subject<Text>,
    scope: &Scope,
) -> Vec<(String, Confidence)> {
    let recognizer = PatternRecognizer::builder()
        .with_walk(walk)
        .build()
        .expect("recognizer builds");
    let ctx = Context::<Text>::new(scope);
    let mut entities = recognizer
        .recognize(&subject, &ctx)
        .await
        .expect("recognize");
    entities.sort_by_key(|e| e.location.range().expect("text range").start);
    let text = subject.data().as_str();
    entities
        .into_iter()
        .map(|e| {
            let range = e.location.range().expect("text range").clone();
            (text[range].to_owned(), e.confidence)
        })
        .collect()
}

/// The names `walk` finds in `text` with no language asserted.
pub async fn names(walk: Walk, text: &str) -> Vec<String> {
    found(walk, text, &[])
        .await
        .into_iter()
        .map(|(name, _)| name)
        .collect()
}
