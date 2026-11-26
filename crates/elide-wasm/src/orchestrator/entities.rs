//! A run's entities as JavaScript reads them: located in their modality, with
//! the text they matched and the trail of how they were found and redacted.
//!
//! The toolkit's report keeps no values, so what each entity matched is read
//! from the documents between detection and redaction, with
//! [`Orchestrator::read`](elide::Orchestrator::read) for what a part's stream
//! holds and from the enrichment artifacts for what OCR or a transcript holds.

use std::collections::HashMap;
use std::ops::Range;

use elide::entity::Entity as RustEntity;
use elide::entity::audit::AuditKind;
use elide::modality::Modality;
use elide::modality::audio::{Audio, Transcription};
use elide::modality::image::{Image, Layout};
use elide::modality::metadata::Metadata;
use elide::modality::tabular::Tabular;
use elide::modality::text::{Text, TextLocation};
use elide::primitive::Confidence;
use elide::{AnalyzedDocument, Document, Orchestrator, PartId, Report};

use super::report::{Entity, Location, Rect, Redaction, Rendered, Source, Span};

/// What an entity matched, read before redaction.
pub(super) struct Matched {
    /// The matched text.
    value: String,
    /// Its span in UTF-16 code units, for an entity in decoded text.
    utf16: Option<Span>,
}

/// What each entity matched, by entity id.
pub(super) type Values = HashMap<String, Matched>;

/// The entities of modality `M` in `report`, each with its part, in part order.
fn entities_of<M: Modality>(report: &Report) -> Vec<(PartId, &RustEntity<M>)> {
    let mut ids: Vec<&PartId> = report
        .part_ids()
        .filter(|(_, name)| *name == M::NAME)
        .map(|(id, _)| id)
        .collect();
    ids.sort_unstable();
    ids.into_iter()
        .filter_map(|id| Some((id, report.part::<M>(id).ok()?.entities())))
        .flat_map(|(id, entities)| entities.iter().map(move |entity| (id.clone(), entity)))
        .collect()
}

/// What every entity of `analyzed` matched in `document`, which detection has
/// not yet changed.
///
/// The values are for display, so reading them never fails a run: an entity
/// whose location cannot be read, or whose part cannot be reached again, has
/// no value.
pub(super) async fn read_matched(
    orchestrator: &Orchestrator,
    document: &mut Document,
    analyzed: &AnalyzedDocument,
) -> Values {
    let mut reader = Reader {
        orchestrator,
        document,
        report: &analyzed.report,
        values: Values::new(),
    };
    reader.text().await;
    reader.values::<Tabular>().await;
    reader.values::<Metadata>().await;
    let mut values = reader.values;
    read_recognized::<Image>(analyzed, Layout::text, &mut values);
    read_recognized::<Audio>(analyzed, Transcription::text, &mut values);
    values
}

/// Reads what the entities of a report matched in the document they were
/// found in, collecting each value by entity id.
struct Reader<'a> {
    orchestrator: &'a Orchestrator,
    document: &'a mut Document,
    report: &'a Report,
    values: Values,
}

/// The text entities that share one text to read: a part's stream, or one page
/// of a rendered one.
struct TextGroup<'r> {
    /// The part the text is in.
    part: PartId,
    /// The page the text is on, for a rendered part.
    page: Option<u32>,
    /// A location of one of the entities, to anchor the read on its page.
    anchor: &'r TextLocation,
    /// The entities and their spans, ascending by start.
    entities: Vec<(&'r RustEntity<Text>, Range<usize>)>,
}

impl Reader<'_> {
    /// What each text entity with a span matched, and the span in UTF-16 code
    /// units.
    ///
    /// Each stream (or page) is read once, from its start to its last entity's end,
    /// and its entities' values are sliced from that text, with one running count
    /// of the UTF-16 units before each.
    async fn text(&mut self) {
        let mut groups: Vec<TextGroup<'_>> = Vec::new();
        let mut index: HashMap<(PartId, Option<u32>), usize> = HashMap::new();
        for (part, entity) in entities_of::<Text>(self.report) {
            // A hidden item flagged whole has no span of its own to read.
            let Some(span) = entity.location().range() else {
                continue;
            };
            let page = entity.location().page();
            let group = *index.entry((part.clone(), page)).or_insert_with(|| {
                groups.push(TextGroup {
                    part,
                    page,
                    anchor: entity.location(),
                    entities: Vec::new(),
                });
                groups.len() - 1
            });
            groups[group].entities.push((entity, span.clone()));
        }
        let locations: Vec<(PartId, TextLocation)> = groups
            .iter_mut()
            .map(|group| {
                group.entities.sort_by_key(|(_, span)| span.start);
                let end = group.entities.iter().map(|(_, span)| span.end).max();
                (
                    group.part.clone(),
                    prefix(group.anchor, group.page, end.unwrap_or(0)),
                )
            })
            .collect();
        let Ok(reads) = self
            .orchestrator
            .read::<Text>(&mut *self.document, &locations)
            .await
        else {
            return;
        };
        for (group, read) in groups.iter().zip(reads) {
            let Ok(text) = read else {
                continue;
            };
            let text = text.as_str();
            let (mut offset, mut units) = (0, 0);
            for (entity, span) in &group.entities {
                let (Some(between), Some(value)) =
                    (text.get(offset..span.start), text.get(span.clone()))
                else {
                    continue;
                };
                units += between.encode_utf16().count();
                offset = span.start;
                self.values.insert(
                    entity.id().to_string(),
                    Matched {
                        value: value.to_owned(),
                        utf16: Some(Span {
                            start: units,
                            end: units + value.encode_utf16().count(),
                        }),
                    },
                );
            }
        }
    }

    /// What each entity of a text-valued modality `M` (a cell, a metadata field)
    /// matched: the value read at its location, which spans only the part it
    /// matched.
    async fn values<M>(&mut self)
    where
        M: Modality<Data: AsRef<str>>,
    {
        let entities = entities_of::<M>(self.report);
        let locations: Vec<(PartId, M::Location)> = entities
            .iter()
            .map(|(part, entity)| (part.clone(), entity.location().clone()))
            .collect();
        let Ok(reads) = self
            .orchestrator
            .read::<M>(&mut *self.document, &locations)
            .await
        else {
            return;
        };
        for ((_, entity), read) in entities.iter().zip(reads) {
            if let Ok(data) = read {
                self.values.insert(
                    entity.id().to_string(),
                    Matched {
                        value: data.as_ref().to_owned(),
                        utf16: None,
                    },
                );
            }
        }
    }
}

/// The text from the start of a stream up to `end`, on `page` for a rendered
/// one.
///
/// A page's text is read by its span and page; the `anchor` entity's regions
/// only keep the location rendered, since a rendered location has at least one.
fn prefix(anchor: &TextLocation, page: Option<u32>, end: usize) -> TextLocation {
    match page {
        Some(page) => TextLocation::rendered(0..end, page, anchor.rects().to_vec()),
        None => TextLocation::inline(0..end),
    }
}

/// What each entity of modality `M` matched in the text its enrichment
/// recognized (an image's OCR text, a clip's transcript), which `text` reads
/// from the part's artifact.
fn read_recognized<M: Modality>(
    analyzed: &AnalyzedDocument,
    text: fn(&M::Artifact) -> &str,
    values: &mut Values,
) {
    for (part, entity) in entities_of::<M>(&analyzed.report) {
        let Some(artifact) = analyzed.artifacts.part::<M>(&part) else {
            continue;
        };
        // An entity found other than by text (a box a vision model drew)
        // matched no text.
        let Some(range) = entity.recognized_range() else {
            continue;
        };
        if let Some(value) = text(artifact).get(range.clone()) {
            values.insert(
                entity.id().to_string(),
                Matched {
                    value: value.to_owned(),
                    utf16: None,
                },
            );
        }
    }
}

/// Every entity of the applied `report`, with what `values` holds of each.
pub(super) fn to_entities(report: &Report, values: Values) -> Vec<Entity> {
    let mut out = Collected {
        report,
        values,
        entities: Vec::new(),
    };
    out.push::<Text>(|entity, utf16| {
        let span = entity.location().range().cloned().unwrap_or_default();
        let rendered = entity.location().page().map(|page| Rendered {
            page,
            rects: entity.location().rects().iter().map(Rect::from).collect(),
        });
        Location::Text {
            start: span.start,
            end: span.end,
            utf16,
            rendered,
        }
    });
    out.push::<Tabular>(|entity, _| Location::Tabular {
        sheet: entity
            .location()
            .sheet_name
            .as_ref()
            .map(ToString::to_string),
        row: entity.location().row_index,
        column: entity.location().column_index,
        span: entity.location().span.clone().map(Span::from),
    });
    out.push::<Image>(|entity, _| {
        let bbox = &entity.location().bounding_box;
        Location::Image {
            x: bbox.min.x,
            y: bbox.min.y,
            width: bbox.width(),
            height: bbox.height(),
        }
    });
    out.push::<Audio>(|entity, _| Location::Audio {
        start_ms: entity.location().span.start_millis(),
        end_ms: entity.location().span.end_millis(),
        speaker: entity
            .location()
            .speaker_id
            .as_ref()
            .map(ToString::to_string),
    });
    out.push::<Metadata>(|entity, _| Location::Metadata {
        key: entity.location().key.to_string(),
        span: entity.location().span.clone().map(Span::from),
    });
    out.entities
}

/// The entities of a report being converted, with what was read of each.
struct Collected<'a> {
    report: &'a Report,
    values: Values,
    entities: Vec<Entity>,
}

impl Collected<'_> {
    /// Append modality `M`'s entities, in part order, each located by
    /// `locate` from the entity and its UTF-16 span.
    fn push<M: Modality>(&mut self, locate: impl Fn(&RustEntity<M>, Option<Span>) -> Location) {
        for (part, entity) in entities_of::<M>(self.report) {
            let id = entity.id().to_string();
            let (value, utf16) = match self.values.remove(&id) {
                Some(matched) => (Some(matched.value), matched.utf16),
                None => (None, None),
            };
            self.entities.push(Entity {
                location: locate(entity, utf16),
                part: part.segments().map(str::to_owned).collect(),
                label: entity.label().as_str().to_owned(),
                value,
                confidence: decimal(entity.confidence()),
                language: entity.language().map(ToString::to_string),
                coref: entity.coref().map(|coref| coref.as_str().to_owned()),
                sources: sources(entity),
                redaction: redaction(entity),
                id,
            });
        }
    }
}

/// Every detection in `entity`'s trail that found it.
fn sources<M: Modality>(entity: &RustEntity<M>) -> Vec<Source> {
    entity
        .audit()
        .events()
        .iter()
        .filter(|event| event.is_recognition())
        .map(|event| Source {
            recognizer: event.source.to_string(),
            model: match &event.kind {
                AuditKind::Model(model) => Some(model.model.name.to_string()),
                _ => None,
            },
            confidence: decimal(event.confidence),
        })
        .collect()
}

/// The redaction applied to `entity`: the operator the applied selection
/// picked. `None` when nothing was applied.
fn redaction<M: Modality>(entity: &RustEntity<M>) -> Option<Redaction> {
    let events = entity.audit().events();
    let applied = events.iter().rev().find_map(|event| match &event.kind {
        AuditKind::Redacted(redaction) => Some(redaction.selection),
        _ => None,
    })?;
    events.iter().find_map(|event| match &event.kind {
        AuditKind::Selected(selection) if event.hash() == applied => Some(Redaction {
            operator: selection.operator.name.to_string(),
            leak_profile: selection.leak_profile.into(),
        }),
        _ => None,
    })
}

/// `confidence` as the decimal it is written as: an `f32` widened to `f64`
/// shows its binary error (`0.949999988…` for `0.95`), so it goes through its
/// shortest decimal form.
fn decimal(confidence: Confidence) -> f64 {
    let confidence = f32::from(confidence);
    confidence
        .to_string()
        .parse()
        .unwrap_or(f64::from(confidence))
}
