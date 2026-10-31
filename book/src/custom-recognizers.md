# Custom recognizers

A recognizer of your own covers what the shipped ones do not: a lookup in
your customer database, a rule that needs code rather than a regex. It
implements `Recognizer<M>`:

- `id` names it, with a version, for the audit trail.
- `recognize` inspects one subject, a chunk of the input, and returns its
  entities in the subject's coordinates. The analyzer moves them to the
  document's.
- `recognize_batch` inspects many subjects; override it when your source can
  answer for several at once.

```rust
use elide::entity::audit::{AuditEvent, PatternEvent};
use elide::modality::text::TextLocation;
use elide::prelude::*;
use elide::primitive::ComponentId;
use elide::recognition::{Context, Subject};

/// Finds the ticket numbers of an internal tracker, such as `OPS-1234`.
struct TicketRecognizer;

#[elide::async_trait]
impl Recognizer<Text> for TicketRecognizer {
    fn id(&self) -> ComponentId {
        ComponentId::new("ticket-recognizer", "1.0.0")
    }

    async fn recognize(&self, subject: &Subject<Text>, _ctx: &Context<'_, Text>) -> Result<Vec<Entity<Text>>> {
        let text = subject.data().as_str();
        let confidence = Confidence::clamped(0.9);

        let entities = text
            .match_indices("OPS-")
            .filter_map(|(start, _)| {
                let digits = text[start + 4..].bytes().take_while(u8::is_ascii_digit).count();
                (digits > 0).then(|| start..start + 4 + digits)
            })
            .filter_map(|span| {
                let location = TextLocation::inline(span.clone());
                let event = AuditEvent::pattern(
                    "ticket-recognizer",
                    confidence,
                    location.clone(),
                    PatternEvent { name: "ops-ticket".into(), ..PatternEvent::default() },
                );
                Entity::builder()
                    .with_label(builtins::INTERNAL_ID.to_ref())
                    .with_location(location)
                    .with_recognized_range(span)
                    .with_event(event)
                    .build()
            })
            .collect();
        Ok(entities)
    }
}

# #[tokio::main]
# async fn main() -> Result<()> {
let analyzer = Analyzer::<Text>::new().with_recognizer(TicketRecognizer);
# let scope = Scope::new().with_catalog(LabelCatalog::with_builtins());
# let found = analyzer.analyze(elide::modality::text::TextData::new("see OPS-1234"), &scope).await?;
# assert_eq!(found.entities.len(), 1);
# Ok(())
# }
```

The recognition event is the first entry of the entity's audit trail: it
records which recognizer found the entity, where, and how sure it was. The
entity's confidence comes from it. The recognized range is where the match
sits in the text the recognizer read; set it on every text match.

The `Context` carries the call's scope: check `ctx.languages(subject)` or the
countries before running a rule that only applies to some of them.

A recognizer over `Text` sees only text. To also run on the text of images
and audio, implement it for every `M: TextRecognizable`, and place spans
with `M::locate`, as the shipped recognizers do.
