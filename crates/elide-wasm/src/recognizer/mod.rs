//! The detection building blocks: a recognizer, ready to fold into a modality's
//! analyzer.
//!
//! A recognizer is compiled once ([`Recognizer::pattern`]) or wired to a JS
//! callback ([`Recognizer::ner`]) and handed to
//! [`Analyzer::recognize`](crate::analyzer::Analyzer::recognize). Both shipped
//! kinds recognize over any [`TextRecognizable`](elide::modality::TextRecognizable)
//! modality, so one handle folds into any text-shaped stage.

mod ner;
mod pattern;

use elide::modality::{Modality, TextRecognizable};
use elide::recognition::Recognizer as RecognizerTrait;
use elide::recognition::context::Enhanced;
use elide::recognition::ner::NerRecognizer;
use elide::recognition::pattern::PatternRecognizer;
use js_sys::Function;
use tsify::Ts;
use wasm_bindgen::prelude::*;

pub use self::pattern::PatternRecognizerConfig;
use crate::error::ElideError;

/// The shipped recognizer kinds, each generic over any text-recognizable
/// modality; the enum lets one opaque handle serve every text-shaped stage
/// without erasing the modality too early.
enum Kind {
    /// A context-enhanced pattern recognizer.
    Pattern(Enhanced<PatternRecognizer>),
    /// A NER recognizer backed by a JS callback.
    Ner(NerRecognizer),
}

/// A compiled recognizer, ready to be folded into a modality's analyzer.
///
/// Built with [`Recognizer::pattern`] or [`Recognizer::ner`] and consumed by
/// [`Analyzer::recognize`](crate::analyzer::Analyzer::recognize). Opaque: the
/// wrapped recognizer is not `Clone`, so each handle is consumed when folded in.
#[wasm_bindgen]
pub struct Recognizer(Kind);

#[wasm_bindgen]
impl Recognizer {
    /// Compile a pattern recognizer from the selected built-in sources.
    ///
    /// # Errors
    ///
    /// Propagates a build error from an invalid shipped rule, which cannot happen
    /// with the built-in set.
    #[wasm_bindgen(js_name = pattern)]
    pub fn pattern(config: Ts<PatternRecognizerConfig>) -> Result<Recognizer, ElideError> {
        Ok(Self(Kind::Pattern(self::pattern::build_pattern(config)?)))
    }

    /// Build a NER recognizer whose inference is a JavaScript `callback`.
    ///
    /// The callback is `(text: string, labels: string[]) => Promise<NerSpan[]>`,
    /// called on each recognition pass, where a `NerSpan` is
    /// `{ label, start, end, score }` with byte offsets into `text`. It runs on
    /// the browser event loop; the recognizer awaits it.
    ///
    /// # Errors
    ///
    /// Propagates a build error from the recognizer configuration.
    #[wasm_bindgen(js_name = ner)]
    pub fn ner(callback: Function) -> Result<Recognizer, ElideError> {
        Ok(Self(Kind::Ner(self::ner::build_ner(callback)?)))
    }
}

impl Recognizer {
    /// Box the recognizer as `Recognizer<M>` for the modality it folds into.
    /// Both kinds recognize over any [`TextRecognizable`] modality.
    pub(crate) fn into_recognizer<M>(self) -> Box<dyn RecognizerTrait<M>>
    where
        M: Modality + TextRecognizable,
    {
        match self.0 {
            Kind::Pattern(r) => Box::new(r),
            Kind::Ner(r) => Box::new(r),
        }
    }
}
