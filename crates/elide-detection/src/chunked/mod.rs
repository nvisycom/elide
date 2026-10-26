//! [`ChunkedRecognizer`]: a [`Recognizer`] decorator that windows an over-long
//! text so a length-limited recognizer sees all of it.

mod chunking;

use std::collections::HashMap;
use std::ops::Range;

use elide_core::Result;
use elide_core::entity::{Entity, LabelRef};
use elide_core::modality::text::{Text, TextData};
use elide_core::primitive::ComponentId;
use elide_core::recognition::{Context, Recognizer, Subject};

pub use self::chunking::{CharWindows, Chunking, Window, WordWindows};

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
/// the exact `(span, label)` duplicates (the same finding seen twice, keeping
/// the higher confidence) and leaves everything else — partial, nested and
/// crossing overlaps, and two different labels on one span — to the downstream
/// reconcile layer, which already fuses and arbitrates it. An entity longer than
/// a whole window can still be split and missed; size the strategy's windows
/// toward the backend's limit if that matters.
///
/// A subject the strategy leaves in a single window (the text fits) is
/// recognized in one call on the original subject, unchanged — hints and
/// enrichment artifact intact. Because it is itself a `Recognizer<Text>`, it
/// drops in anywhere a text recognizer goes and composes with the other
/// decorators.
///
/// Compose it *closest to* the length-limited recognizer: wrap the bare
/// recognizer (NER, pattern) and layer context enhancement outside, as
/// `Enhanced::new(ChunkedRecognizer::new(ner), …)`. The bare recognizers read
/// only the chunk text, so windowing is lossless for them, and the outer
/// `Enhanced` sees the whole subject with its hints and tokens. Wrapping the
/// other way (`ChunkedRecognizer<Enhanced<_>>`) enhances inside each window:
/// hints still apply (they are carried per window), but lemma-aware matching
/// degrades to surface text, since a window carries no re-tokenized artifact.
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
        //
        // The subject's hints are chunk-global context (a column header, an
        // object key — content matched against the whole chunk, not indexed by a
        // body-text offset), so each window carries all of them unchanged; a
        // context recognizer like `Enhanced` wrapped *inside* the chunker then
        // still sees them. The enrichment artifact is left unset: its coordinates
        // (producer tokens) are body-text-local and would be wrong in a window
        // slice, and re-tokenizing a window is an enricher's job, not a chunker's
        // — a context recognizer degrades to surface-text matching inside a
        // window. The intended composition keeps that moot: wrap the
        // length-limited recognizer and enhance *outside* the chunker, so
        // `Enhanced` sees the whole subject with its artifact intact.
        let subjects: Vec<Subject<Text>> = windows
            .iter()
            .map(|w| {
                Subject::new(TextData::new(text[w.span()].to_owned()))
                    .with_hints(subject.hints.clone())
            })
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

/// Collapse entities that are the *same finding* seen twice — identical
/// `(span, label)` — keeping the highest-confidence one of each group.
///
/// Keying on the label as well as the span is deliberate: an exact overlap
/// duplicate (the same text scanned by two windows) carries the same label, so
/// this removes it; but two *different* labels reported on one span (a span a
/// recognizer calls both `PERSON` and `ORG`) is a genuine cross-label conflict,
/// which the reconcile layer's cross-label pass exists to arbitrate — dropping
/// the lower-confidence label here would pre-empt it and silently lose a
/// candidate. Entities with no span (none, in practice, since the inner scans
/// plain text) are kept as-is. Partial, nested and crossing overlaps are also
/// left for the reconcile layer.
fn dedupe_exact(entities: Vec<Entity<Text>>) -> Vec<Entity<Text>> {
    let mut best: HashMap<(Range<usize>, LabelRef), usize> = HashMap::new();
    let mut kept: Vec<Option<Entity<Text>>> = Vec::with_capacity(entities.len());

    for entity in entities {
        let Some(span) = entity.location.range().cloned() else {
            kept.push(Some(entity));
            continue;
        };
        let key = (span, entity.label.clone());
        match best.get(&key) {
            Some(&idx)
                if kept[idx]
                    .as_ref()
                    .is_some_and(|e| e.confidence >= entity.confidence) =>
            {
                // An equal-or-better duplicate is already kept; drop this one.
            }
            Some(&idx) => {
                kept[idx] = None;
                best.insert(key, kept.len());
                kept.push(Some(entity));
            }
            None => {
                best.insert(key, kept.len());
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
        // A single window means the strategy did not split: the text fits. Run
        // the inner recognizer directly on the *original* subject, which is the
        // within-limit case the type promises is "recognized in one call,
        // unchanged" — it avoids the per-window copy and the (no-op) rebase, and
        // keeps the subject's hints and enrichment artifact, which a
        // reconstructed window subject would drop. Any content outside the one
        // window is inter-word whitespace the window omits (e.g. a trailing
        // newline), never a recognizable region, so delegating the whole subject
        // can only find the same entities or more — never fewer.
        if let [_only] = windows.as_slice() {
            return self.inner.recognize(subject, ctx).await;
        }
        self.recognize_windowed(subject, &windows, ctx).await
    }
}

#[cfg(test)]
mod tests {
    use elide_core::entity::audit::{AuditEvent, ModelEvent};
    use elide_core::entity::{Entity, EntityBuilder, LabelRef};
    use elide_core::modality::ResolvedHint;
    use elide_core::modality::text::TextLocation;
    use elide_core::primitive::Confidence;
    use elide_core::recognition::{Scope, Subject};

    use super::*;

    /// A recognizer that reports a match at a fixed substring, wherever it occurs
    /// in the text it is given, under each of its `labels`, at a configured
    /// confidence. It records the texts it was asked to scan, so a test can
    /// assert how the input was windowed.
    struct FindsSubstring {
        needle: &'static str,
        labels: &'static [&'static str],
        confidence: f32,
        seen: std::sync::Mutex<Vec<String>>,
    }

    impl FindsSubstring {
        /// Reports one entity (label `THING`) per match.
        fn new(needle: &'static str, confidence: f32) -> Self {
            Self::labelled(needle, &["THING"], confidence)
        }

        /// Reports one entity per label per match, all on the same span.
        fn labelled(
            needle: &'static str,
            labels: &'static [&'static str],
            confidence: f32,
        ) -> Self {
            Self {
                needle,
                labels,
                confidence,
                seen: std::sync::Mutex::new(Vec::new()),
            }
        }

        fn entity_at(&self, label: &str, span: Range<usize>) -> Entity<Text> {
            let location = TextLocation::inline(span.clone());
            let confidence = Confidence::new(self.confidence).unwrap();
            let event = AuditEvent::model(
                "finds-substring",
                confidence,
                location.clone(),
                ModelEvent::default(),
            );
            EntityBuilder::new()
                .with_label(LabelRef::new(label))
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
                .flat_map(|(start, m)| {
                    self.labels
                        .iter()
                        .map(move |label| self.entity_at(label, start..start + m.len()))
                })
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
    async fn trailing_whitespace_still_takes_the_single_scan_path() {
        // A trailing newline means the single word window (first word start to
        // last word end) does not span the whole text. The fast path must still
        // fire — a single window means no split — delegating the *original*
        // subject so its hints and artifact survive, rather than rebuilding a
        // window subject from a slice.
        let recognizer = ChunkedRecognizer::with_chunking(
            FindsSubstring::new("Jane", 0.9),
            WordWindows::new(8, 2),
        );
        let scope = Scope::new();
        let ctx = Context::new(&scope);
        let out = recognizer
            .recognize(&subject("Jane paid Jane\n"), &ctx)
            .await
            .unwrap();
        assert_eq!(out.len(), 2);
        // One scan, and it saw the original text verbatim (the trailing newline
        // proves the original subject was delegated, not a reslice).
        let seen = recognizer.inner().seen.lock().unwrap();
        assert_eq!(seen.as_slice(), ["Jane paid Jane\n"]);
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

    #[tokio::test]
    async fn two_labels_on_one_span_both_survive_the_windowed_path() {
        // A recognizer reporting the same span under two labels is a cross-label
        // conflict for the reconcile layer, not an overlap duplicate. The
        // windowed dedup keys on (span, label), so both labels survive even when
        // the span sits in a window overlap (so each window reports both).
        let text = "a b c NEEDLE d e f g h i";
        let recognizer = ChunkedRecognizer::with_chunking(
            FindsSubstring::labelled("NEEDLE", &["PERSON", "ORG"], 0.8),
            WordWindows::new(4, 2),
        );
        let scope = Scope::new();
        let ctx = Context::new(&scope);
        let out = recognizer.recognize(&subject(text), &ctx).await.unwrap();

        let needle_at = text.find("NEEDLE").unwrap();
        // Exactly the two distinct labels, each once (overlap duplicates of each
        // label collapsed), both at the rebased span.
        assert_eq!(out.len(), 2);
        assert!(
            out.iter()
                .all(|e| e.location.range() == Some(&(needle_at..needle_at + 6)))
        );
        let mut labels: Vec<&str> = out.iter().map(|e| e.label.as_str()).collect();
        labels.sort_unstable();
        assert_eq!(labels, ["ORG", "PERSON"]);
    }

    /// Records how many hints each window subject carried, to assert chunk-global
    /// context survives the split.
    struct RecordsHints {
        per_window_hint_count: std::sync::Mutex<Vec<usize>>,
    }

    #[async_trait::async_trait]
    impl Recognizer<Text> for RecordsHints {
        fn id(&self) -> ComponentId {
            ComponentId::new("records-hints", "0")
        }

        async fn recognize(
            &self,
            subject: &Subject<Text>,
            _ctx: &Context<'_, Text>,
        ) -> Result<Vec<Entity<Text>>> {
            self.per_window_hint_count
                .lock()
                .unwrap()
                .push(subject.hints.len());
            Ok(Vec::new())
        }
    }

    #[tokio::test]
    async fn hints_are_carried_into_every_window() {
        // A subject's hints are chunk-global context; a split must not drop them,
        // or a context recognizer wrapped inside the chunker loses its boosts.
        let text = "w0 w1 w2 w3 w4 w5 w6 w7 w8 w9"; // 10 words -> 3 windows at (4,1)
        let hint = ResolvedHint::new(
            TextLocation::inline(0..2),
            TextData::new("header".to_owned()),
        );
        let subject = Subject::new(TextData::new(text.to_owned())).with_hints(vec![hint]);
        let recognizer = ChunkedRecognizer::with_chunking(
            RecordsHints {
                per_window_hint_count: std::sync::Mutex::new(Vec::new()),
            },
            WordWindows::new(4, 1),
        );
        let scope = Scope::new();
        let ctx = Context::new(&scope);
        recognizer.recognize(&subject, &ctx).await.unwrap();

        // It really windowed, and every window carried the one hint.
        let counts = recognizer.inner().per_window_hint_count.lock().unwrap();
        assert!(counts.len() > 1, "the text was split into several windows");
        assert!(counts.iter().all(|&n| n == 1), "each window kept the hint");
    }
}
