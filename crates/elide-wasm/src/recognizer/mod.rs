//! The detection building blocks: a recognizer, ready to fold into a modality's
//! analyzer on the [`PipelineBuilder`](crate::pipeline::PipelineBuilder).
//!
//! A recognizer is compiled once (patterns) or wired to a JS callback (NER) and
//! handed to a modality's `with_*` method. Both shipped kinds recognize over any
//! [`TextRecognizable`](elide::modality::TextRecognizable) modality, so one
//! handle folds into either the text or the tabular stage.

mod ner;
mod pattern;

use elide::modality::text::Text;
use elide::modality::{Modality, TextRecognizable};
use elide::recognition::Recognizer;
use elide::recognition::context::Enhanced;
use elide::recognition::ner::NerRecognizer;
use elide::recognition::pattern::PatternRecognizer;
use wasm_bindgen::prelude::*;

pub use self::ner::create_ner_recognizer;
pub use self::pattern::create_pattern_recognizer;

/// The shipped recognizer kinds, each generic over any text-recognizable
/// modality; the enum lets one opaque handle serve both the text and tabular
/// stages without erasing the modality too early.
enum Kind {
    /// A context-enhanced pattern recognizer.
    Pattern(Enhanced<PatternRecognizer>),
    /// A NER recognizer backed by a JS callback.
    Ner(NerRecognizer),
}

/// A compiled recognizer, ready to be folded into a modality's analyzer.
///
/// Opaque: the wrapped recognizer is not `Clone`, so a modality's `with_*`
/// method consumes each handle it is given. Reusing one afterwards throws a
/// null-pointer error.
#[wasm_bindgen]
pub struct RecognizerHandle(Kind);

impl RecognizerHandle {
    /// Wrap a context-enhanced pattern recognizer.
    pub(crate) fn pattern(recognizer: Enhanced<PatternRecognizer>) -> Self {
        Self(Kind::Pattern(recognizer))
    }

    /// Wrap a NER recognizer.
    pub(crate) fn ner(recognizer: NerRecognizer) -> Self {
        Self(Kind::Ner(recognizer))
    }

    /// Box the recognizer as `Recognizer<M>` for the modality it folds into.
    /// Both kinds recognize over any [`TextRecognizable`] modality.
    pub(crate) fn into_recognizer<M>(self) -> Box<dyn Recognizer<M>>
    where
        M: Modality + TextRecognizable,
    {
        match self.0 {
            Kind::Pattern(r) => Box::new(r),
            Kind::Ner(r) => Box::new(r),
        }
    }

    /// Box the recognizer for the text stage.
    pub(crate) fn into_text(self) -> Box<dyn Recognizer<Text>> {
        self.into_recognizer()
    }
}
