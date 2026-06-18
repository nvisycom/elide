//! Assembles the detection side of the pipeline: a three-recognizer
//! [`Analyzer`] plus its deduplication layers.

use veil_core::Result;
use veil_core::entity::builtins;
use veil_core::modality::text::Text;
use veil_core::primitive::ConfidenceThreshold;

use veil_llm::LlmRecognizer;
use veil_ner::NerRecognizer;
use veil_pattern::PatternRecognizer;

use veil_toolkit::Analyzer;
use veil_toolkit::deduplication::filter::FilterLayer;
use veil_toolkit::deduplication::fuse::{FuseLayer, MaxConfidence};
use veil_toolkit::deduplication::resolve::{HighestConfidence, ResolveLayer};

/// Build the three-recognizer analyzer plus its deduplication pipeline.
pub fn build_analyzer() -> Result<Analyzer<Text>> {
    // Real built-in patterns + dictionaries, with context boosting.
    let patterns = PatternRecognizer::builder()
        .with_builtin_patterns()
        .with_builtin_dictionaries()
        .build_context_enhanced()?;

    // Mock NER: wired like a real model, returns no entities offline.
    let ner = NerRecognizer::builder()
        .with_name("ner-mock")
        .with_mock_backend()
        .with_supported_labels(vec![
            builtins::PERSON_NAME.to_ref(),
            builtins::ADDRESS.to_ref(),
        ])
        .build()?;

    // Mock LLM: wired like a real model, returns no entities offline.
    let llm = LlmRecognizer::<Text>::builder()
        .with_name("llm-mock")
        .with_mock_backend()
        .with_default_prompt()
        .build()?;

    Ok(Analyzer::new()
        .with_recognizer(patterns)
        .with_recognizer(ner)
        .with_recognizer(llm)
        .with_layer(FuseLayer::new(MaxConfidence))
        .with_layer(ResolveLayer::new(HighestConfidence))
        .with_layer(FilterLayer::new().with_threshold(ConfidenceThreshold::BASELINE)))
}
