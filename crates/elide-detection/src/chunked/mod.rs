//! [`ChunkedRecognizer`]: a [`Recognizer`] decorator that windows an over-long
//! text so a length-limited recognizer sees all of it.

mod chunking;

use std::collections::HashMap;
use std::ops::Range;

use elide_core::Result;
use elide_core::entity::Entity;
use elide_core::modality::text::{Text, TextData};
use elide_core::primitive::ComponentId;
use elide_core::recognition::{Context, Recognizer, Subject};

pub use self::chunking::{Chunking, Window, WordWindows};

/// Wraps a text [`Recognizer`] whose backend caps input length: a [`Chunking`]
/// strategy splits an over-long subject into overlapping windows, each is
/// recognized on the inner recognizer, and the findings are rebased onto the
/// full text and merged.
///
/// A hosted span recognizer (GLiNER and the like) accepts only so many tokens
/// per call, so a long chunk silently loses everything past the limit. This
/// decorator asks its `chunking` for the windows (word count by default, via
/// [`WordWindows`]), recognizes each window, translates every finding's
/// coordinates back to the full text with [`Entity::offset`], and returns one
/// combined list.
///
/// The windows overlap, so a window edge can cut an entity in two but the entity
/// still appears whole in a neighbour. The flip side is that the overlap region
/// is scanned twice, so an entity there is found in both windows; this collapses
/// the exact `(start, end)` duplicates (keeping the higher confidence) and
/// leaves any remaining overlap — partial, nested, crossing — to the downstream
/// reconcile layer, which already fuses it. An entity longer than a whole window
/// can still be split and missed; size the strategy's windows toward the
/// backend's limit if that matters.
///
/// A subject the strategy leaves in a single whole window is recognized in one
/// call, unchanged. Because it is itself a `Recognizer<Text>`, it drops in
/// anywhere a text recognizer goes and composes with the other decorators.
pub struct ChunkedRecognizer<R, C = WordWindows> {
    inner: R,
    chunking: C,
}

impl<R> ChunkedRecognizer<R, WordWindows> {
    /// Window `inner` with the default [`WordWindows`] strategy (the GLiNER2
    /// long-document setting: 384-word windows overlapping by 64 words).
    #[must_use]
    pub fn new(inner: R) -> Self {
        Self::with_chunking(inner, WordWindows::default())
    }
}

impl<R, C> ChunkedRecognizer<R, C> {
    /// Window `inner` with an explicit [`Chunking`] strategy.
    #[must_use]
    pub fn with_chunking(inner: R, chunking: C) -> Self {
        Self { inner, chunking }
    }

    /// The wrapped recognizer.
    pub fn inner(&self) -> &R {
        &self.inner
    }
}

impl<R: Recognizer<Text>, C: Chunking> ChunkedRecognizer<R, C> {
    /// Recognize `subject` by windowing its text: run each window on the inner
    /// recognizer (coalesced into one `recognize_batch`), rebase every finding
    /// onto the full text, and collapse exact duplicates from the overlaps.
    async fn recognize_windowed(
        &self,
        subject: &Subject<Text>,
        windows: &[Window],
        ctx: &Context<'_, Text>,
    ) -> Result<Vec<Entity<Text>>> {
        let text = subject.data().as_str();

        // One subject per window over a real slice of the original text, so byte
        // offsets within a window are the full text's offsets minus the window
        // start — the exact delta `Entity::offset` adds back.
        let subjects: Vec<Subject<Text>> = windows
            .iter()
            .map(|w| Subject::new(TextData::new(text[w.span()].to_owned())))
            .collect();
        let per_window = self.inner.recognize_batch(&subjects, ctx).await?;

        let mut found = Vec::new();
        for (window, entities) in windows.iter().zip(per_window) {
            for entity in entities {
                // A window-local finding rebases onto the window's start in the
                // full text. A location with no span (a mark) can't be rebased;
                // the inner recognizer scanned plain text, so it does not produce
                // one, and such an entity is dropped rather than mis-placed.
                if let Some(entity) = entity.offset(window.start) {
                    found.push(entity);
                }
            }
        }
        Ok(dedupe_exact(found))
    }
}

/// Collapse entities sharing an identical `(start, end)` span, keeping the
/// highest-confidence one of each group. Entities with no span (none, in
/// practice, since the inner scans plain text) are kept as-is. Partial, nested
/// and crossing overlaps are left for the reconcile layer.
fn dedupe_exact(entities: Vec<Entity<Text>>) -> Vec<Entity<Text>> {
    let mut best: HashMap<Range<usize>, usize> = HashMap::new();
    let mut kept: Vec<Option<Entity<Text>>> = Vec::with_capacity(entities.len());

    for entity in entities {
        let Some(span) = entity.location.range().cloned() else {
            kept.push(Some(entity));
            continue;
        };
        match best.get(&span) {
            Some(&idx)
                if kept[idx]
                    .as_ref()
                    .is_some_and(|e| e.confidence >= entity.confidence) =>
            {
                // An equal-or-better duplicate is already kept; drop this one.
            }
            Some(&idx) => {
                kept[idx] = None;
                best.insert(span, kept.len());
                kept.push(Some(entity));
            }
            None => {
                best.insert(span, kept.len());
                kept.push(Some(entity));
            }
        }
    }
    kept.into_iter().flatten().collect()
}

#[async_trait::async_trait]
impl<R: Recognizer<Text>, C: Chunking> Recognizer<Text> for ChunkedRecognizer<R, C> {
    fn id(&self) -> ComponentId {
        self.inner.id()
    }

    async fn recognize(
        &self,
        subject: &Subject<Text>,
        ctx: &Context<'_, Text>,
    ) -> Result<Vec<Entity<Text>>> {
        let text = subject.data().as_str();
        let windows = self.chunking.windows(text);
        // A single window covering the whole text is the within-limit case: run
        // the inner recognizer directly on the original subject, avoiding the
        // per-window copy and the (no-op) rebase. (It also keeps the subject's
        // hints, which a reconstructed window subject would drop.)
        if let [only] = windows.as_slice()
            && only.span() == (0..text.len())
        {
            return self.inner.recognize(subject, ctx).await;
        }
        self.recognize_windowed(subject, &windows, ctx).await
    }
}

#[cfg(test)]
mod tests {
    use elide_core::entity::audit::{AuditEvent, ModelEvent};
    use elide_core::entity::{Entity, EntityBuilder, LabelRef};
    use elide_core::modality::text::TextLocation;
    use elide_core::primitive::Confidence;
    use elide_core::recognition::{Scope, Subject};

    use super::*;

    /// A recognizer that reports a match at a fixed substring, wherever it occurs
    /// in the text it is given, at a configured confidence. It records the texts
    /// it was asked to scan, so a test can assert how the input was windowed.
    struct FindsSubstring {
        needle: &'static str,
        confidence: f32,
        seen: std::sync::Mutex<Vec<String>>,
    }

    impl FindsSubstring {
        fn new(needle: &'static str, confidence: f32) -> Self {
            Self {
                needle,
                confidence,
                seen: std::sync::Mutex::new(Vec::new()),
            }
        }

        fn entity_at(&self, span: Range<usize>) -> Entity<Text> {
            let location = TextLocation::inline(span.clone());
            let confidence = Confidence::new(self.confidence).unwrap();
            let event = AuditEvent::model(
                "finds-substring",
                confidence,
                location.clone(),
                ModelEvent::default(),
            );
            EntityBuilder::new()
                .with_label(LabelRef::new("THING"))
                .with_location(location)
                .with_confidence(confidence)
                .with_recognized_range(span)
                .with_event(event)
                .build()
                .expect("fields provided")
        }
    }

    #[async_trait::async_trait]
    impl Recognizer<Text> for FindsSubstring {
        fn id(&self) -> ComponentId {
            ComponentId::new("finds-substring", "0")
        }

        async fn recognize(
            &self,
            subject: &Subject<Text>,
            _ctx: &Context<'_, Text>,
        ) -> Result<Vec<Entity<Text>>> {
            let text = subject.data().as_str();
            self.seen.lock().unwrap().push(text.to_owned());
            let found = text
                .match_indices(self.needle)
                .map(|(start, m)| self.entity_at(start..start + m.len()))
                .collect();
            Ok(found)
        }
    }

    fn subject(text: &str) -> Subject<Text> {
        Subject::new(TextData::new(text.to_owned()))
    }

    #[tokio::test]
    async fn a_short_text_is_recognized_in_one_call() {
        let recognizer = ChunkedRecognizer::with_chunking(
            FindsSubstring::new("Jane", 0.9),
            WordWindows::new(8, 2),
        );
        let scope = Scope::new();
        let ctx = Context::new(&scope);
        let out = recognizer
            .recognize(&subject("Jane paid Jane"), &ctx)
            .await
            .unwrap();
        // Both occurrences, one scan of the whole text.
        assert_eq!(out.len(), 2);
        assert_eq!(recognizer.inner().seen.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_long_text_is_windowed_and_findings_are_rebased() {
        // 10 words, size 4, overlap 1 -> step 3 -> windows [0..4][3..7][6..10][9..10].
        let text = "w0 w1 w2 NEEDLE w4 w5 w6 w7 w8 w9";
        let recognizer = ChunkedRecognizer::with_chunking(
            FindsSubstring::new("NEEDLE", 0.9),
            WordWindows::new(4, 1),
        );
        let scope = Scope::new();
        let ctx = Context::new(&scope);
        let out = recognizer.recognize(&subject(text), &ctx).await.unwrap();

        // NEEDLE is the 4th word; it must be found and rebased to its real offset
        // in the full text, regardless of which window saw it.
        let needle_at = text.find("NEEDLE").unwrap();
        assert_eq!(out.len(), 1, "exact duplicates from overlap are collapsed");
        assert_eq!(out[0].location.range(), Some(&(needle_at..needle_at + 6)));
        assert_eq!(out[0].recognized_range, Some(needle_at..needle_at + 6));
        // It really was windowed (more than one scan).
        assert!(recognizer.inner().seen.lock().unwrap().len() > 1);
    }

    #[tokio::test]
    async fn an_entity_in_the_overlap_is_deduped_keeping_higher_confidence() {
        // Place NEEDLE where two windows overlap so both report it; the collapse
        // keeps one. (Both report the same confidence here; the point is one
        // survives, at the correct rebased offset.)
        let text = "a b c NEEDLE d e f g h i";
        let recognizer = ChunkedRecognizer::with_chunking(
            FindsSubstring::new("NEEDLE", 0.8),
            WordWindows::new(4, 2),
        );
        let scope = Scope::new();
        let ctx = Context::new(&scope);
        let out = recognizer.recognize(&subject(text), &ctx).await.unwrap();
        let needle_at = text.find("NEEDLE").unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].location.range(), Some(&(needle_at..needle_at + 6)));
    }
}
