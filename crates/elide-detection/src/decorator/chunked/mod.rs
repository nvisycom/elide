//! A [`Recognizer`] decorator that windows text too long for its recognizer.
//!
//! [`ChunkedRecognizer`] splits an over-long text so a length-limited
//! recognizer sees all of it.

mod chunking;

use std::collections::HashMap;
use std::mem;
use std::ops::Range;

use elide_core::Result;
use elide_core::entity::{Entity, LabelRef};
use elide_core::modality::text::{Text, TextData};
use elide_core::primitive::ComponentId;
use elide_core::recognition::{Context, Recognizer, Subject};

pub use self::chunking::{CharWindows, Chunking, Window, WordWindows};
use crate::batch::expect_one_per_input;

/// Wraps a text [`Recognizer`] whose backend caps input length, windowing text.
///
/// A [`Chunking`] strategy splits an over-long subject into overlapping
/// windows, each is recognized on the inner recognizer, and the findings are
/// rebased onto the full text and merged.
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
/// Wrap the length-limited recognizer itself (a NER model, say). A recognizer
/// that reads only the text loses nothing to windowing. One that also reads
/// the subject's hints still sees them, since every window carries them, but
/// one that reads the enrichment artifact (tokens) gets none inside a window.
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

/// Where one entry of the inner batch came from.
enum Entry {
    /// A subject that fits in one window, recognized as itself.
    Whole { subject: usize },
    /// One window of a longer subject; `part` indexes the window subjects.
    Window {
        subject: usize,
        window: Window,
        part: usize,
    },
}

/// The window `window` of `subject` as a subject of its own: its slice of the
/// text, every hint, and the language claims that reach into it, shifted into
/// the window's coordinates.
///
/// The hints are chunk-global context (a column header, an object key, matched
/// against the whole chunk rather than indexed by a body-text offset), so each
/// window carries all of them. The enrichment artifact is left out: its
/// coordinates (producer tokens) are the whole text's and would be wrong in a
/// window, and re-tokenizing a window is an enricher's job, not a chunker's.
fn window_subject(subject: &Subject<Text>, window: Window) -> Subject<Text> {
    let text = &subject.data().as_str()[window.span()];
    let mut part =
        Subject::new(TextData::new(text.to_owned())).with_hints(subject.hints().to_vec());
    for claim in subject.detected_languages() {
        if let Some(claim) = claim.within(window.span()) {
            part.detect_language(claim);
        }
    }
    part
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
        ctx: &Context<'_>,
    ) -> Result<Vec<Entity<Text>>> {
        let mut found = self.recognize_batch(&[subject], ctx).await?;
        Ok(found.pop().unwrap_or_default())
    }

    /// Recognize every subject in one inner batch: a subject the strategy keeps
    /// in a single window as itself (hints, languages, and artifact intact), a
    /// longer one as its windows. A window's findings are rebased onto its
    /// subject's text and the exact duplicates the overlaps produce collapsed.
    ///
    /// A single window means the text fits: anything outside it is inter-word
    /// whitespace the window omits (a trailing newline, say), never a
    /// recognizable region, so recognizing the whole subject finds the same
    /// entities or more, never fewer.
    async fn recognize_batch(
        &self,
        subjects: &[&Subject<Text>],
        ctx: &Context<'_>,
    ) -> Result<Vec<Vec<Entity<Text>>>> {
        let mut entries = Vec::new();
        let mut parts = Vec::new();
        for (index, subject) in subjects.iter().enumerate() {
            match self.chunking.windows(subject.data().as_str()).as_slice() {
                [] => {}
                [_] => entries.push(Entry::Whole { subject: index }),
                windows => {
                    for &window in windows {
                        entries.push(Entry::Window {
                            subject: index,
                            window,
                            part: parts.len(),
                        });
                        parts.push(window_subject(subject, window));
                    }
                }
            }
        }

        if entries.is_empty() {
            return Ok(vec![Vec::new(); subjects.len()]);
        }
        let batch: Vec<&Subject<Text>> = entries
            .iter()
            .map(|entry| match *entry {
                Entry::Whole { subject } => subjects[subject],
                Entry::Window { part, .. } => &parts[part],
            })
            .collect();
        let found = self.inner.recognize_batch(&batch, ctx).await?;
        let found = expect_one_per_input(self.inner.id(), found, batch.len())?;

        let mut per_subject: Vec<Vec<Entity<Text>>> = vec![Vec::new(); subjects.len()];
        let mut windowed = vec![false; subjects.len()];
        for (entry, entities) in entries.iter().zip(found) {
            match *entry {
                Entry::Whole { subject } => per_subject[subject] = entities,
                Entry::Window {
                    subject, window, ..
                } => {
                    // A window-local finding rebases onto the window's start. A
                    // location with no span (a mark) can't be rebased; the inner
                    // recognizer scanned plain text, so it does not produce one,
                    // and such an entity is dropped rather than mis-placed.
                    windowed[subject] = true;
                    per_subject[subject].extend(
                        entities
                            .into_iter()
                            .filter_map(|entity| entity.offset(window.start)),
                    );
                }
            }
        }
        for (entities, windowed) in per_subject.iter_mut().zip(windowed) {
            if windowed {
                *entities = dedupe_exact(mem::take(entities));
            }
        }
        Ok(per_subject)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use elide_core::entity::audit::{AuditEvent, ModelId};
    use elide_core::entity::{Entity, LabelRef};
    use elide_core::modality::ResolvedHint;
    use elide_core::modality::text::TextLocation;
    use elide_core::primitive::{Confidence, LanguageClaim, LanguageTag};
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
        seen: Mutex<Vec<String>>,
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
                seen: Mutex::new(Vec::new()),
            }
        }

        fn entity_at(&self, label: &str, span: Range<usize>) -> Entity<Text> {
            let location = TextLocation::inline(span.clone());
            let confidence = Confidence::new(self.confidence).unwrap();
            let event = AuditEvent::model(
                "finds-substring",
                confidence,
                location.clone(),
                ModelId::default(),
            );
            Entity::new(LabelRef::new(label), location)
                .with_recognized_range(span)
                .with_event(event)
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
            _ctx: &Context<'_>,
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

    /// One batch as a [`Recorder`] saw it: each subject's text and language
    /// claims.
    type Batch = Vec<(String, Vec<LanguageClaim>)>;

    /// A recognizer that finds `needle` and records each batch it is handed.
    #[derive(Default)]
    struct Recorder {
        batches: Mutex<Vec<Batch>>,
    }

    #[async_trait::async_trait]
    impl Recognizer<Text> for Recorder {
        fn id(&self) -> ComponentId {
            ComponentId::new("recorder", "0")
        }

        async fn recognize(
            &self,
            subject: &Subject<Text>,
            ctx: &Context<'_>,
        ) -> Result<Vec<Entity<Text>>> {
            let mut found = self.recognize_batch(&[subject], ctx).await?;
            Ok(found.pop().unwrap_or_default())
        }

        async fn recognize_batch(
            &self,
            subjects: &[&Subject<Text>],
            ctx: &Context<'_>,
        ) -> Result<Vec<Vec<Entity<Text>>>> {
            self.batches.lock().unwrap().push(
                subjects
                    .iter()
                    .map(|s| {
                        (
                            s.data().as_str().to_owned(),
                            s.detected_languages().to_vec(),
                        )
                    })
                    .collect(),
            );
            let finder = FindsSubstring::new("needle", 0.9);
            let mut found = Vec::new();
            for subject in subjects {
                found.push(finder.recognize(subject, ctx).await?);
            }
            Ok(found)
        }
    }

    /// Each entity's span, as `(start, end)`.
    fn spans(entities: &[Entity<Text>]) -> Vec<(usize, usize)> {
        entities
            .iter()
            .filter_map(|entity| entity.location.range())
            .map(|range| (range.start, range.end))
            .collect()
    }

    /// A short and a long subject go to the inner recognizer as one batch: the
    /// short one as itself, the long one as its windows, and each subject gets
    /// back its own findings in its own coordinates.
    #[tokio::test]
    async fn a_mixed_batch_is_one_inner_call() {
        let recorder = Arc::new(Recorder::default());
        let chunked =
            ChunkedRecognizer::with_chunking(Arc::clone(&recorder), WordWindows::new(3, 1));
        let short = subject("a needle");
        let long = subject("w0 w1 w2 w3 needle w5");
        let scope = Scope::new();
        let ctx = Context::new(&scope);

        let found = chunked
            .recognize_batch(&[&short, &long], &ctx)
            .await
            .unwrap();

        let batches = recorder.batches.lock().unwrap();
        assert_eq!(batches.len(), 1, "one inner call for the whole batch");
        let texts: Vec<&str> = batches[0].iter().map(|(text, _)| text.as_str()).collect();
        assert_eq!(texts, ["a needle", "w0 w1 w2", "w2 w3 needle", "needle w5"]);
        assert_eq!(spans(&found[0]), [(2, 8)]);
        // The needle at 12..18 of the long text, seen in two windows, once.
        assert_eq!(spans(&found[1]), [(12, 18)]);
    }

    /// Each window carries the subject's language claims: a whole-text claim as
    /// it is, a span claim clipped to the window and shifted to its start.
    #[tokio::test]
    async fn windows_carry_the_language_claims_that_reach_them() {
        let recorder = Arc::new(Recorder::default());
        let chunked =
            ChunkedRecognizer::with_chunking(Arc::clone(&recorder), WordWindows::new(2, 0));
        let en = LanguageTag::parse("en").unwrap();
        let uk = LanguageTag::parse("uk").unwrap();
        let mut long = subject("aa bb cc dd");
        long.detect_language(LanguageClaim::asserted(en.clone()));
        // "cc dd" (6..11) is Ukrainian: only the second window holds it.
        long.detect_language(LanguageClaim::asserted(uk.clone()).with_span(6..11));
        let scope = Scope::new();
        let ctx = Context::new(&scope);

        chunked.recognize_batch(&[&long], &ctx).await.unwrap();

        let batches = recorder.batches.lock().unwrap();
        let windows = &batches[0];
        assert_eq!(windows[0].0, "aa bb");
        assert_eq!(windows[0].1, [LanguageClaim::asserted(en.clone())]);
        assert_eq!(windows[1].0, "cc dd");
        assert_eq!(
            windows[1].1,
            [
                LanguageClaim::asserted(en),
                LanguageClaim::asserted(uk).with_span(0..5)
            ]
        );
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
        per_window_hint_count: Mutex<Vec<usize>>,
    }

    #[async_trait::async_trait]
    impl Recognizer<Text> for RecordsHints {
        fn id(&self) -> ComponentId {
            ComponentId::new("records-hints", "0")
        }

        async fn recognize(
            &self,
            subject: &Subject<Text>,
            _ctx: &Context<'_>,
        ) -> Result<Vec<Entity<Text>>> {
            self.per_window_hint_count
                .lock()
                .unwrap()
                .push(subject.hints().len());
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
                per_window_hint_count: Mutex::new(Vec::new()),
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
