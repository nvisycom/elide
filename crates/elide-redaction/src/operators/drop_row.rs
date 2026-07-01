//! [`DropRow`]: drop the entire row a matched tabular entity sits in.

use elide_core::Result;
use elide_core::entity::Entity;
use elide_core::modality::tabular::{Tabular, TabularReplacement};
use elide_core::modality::text::TextData;
use elide_core::operator::{LeakProfile, Operator, OperatorId};

/// Drop the entire row a matched entity sits in.
///
/// A structural treatment, not a cell edit: where [`Erase`] blanks one
/// cell's text, `DropRow` removes the whole record. Useful for "this row
/// names a sanctioned individual — drop it entirely". Any match in a row
/// drops that row, so the table shrinks by one record per matched row.
///
/// The header row is never dropped — that would strip the table's schema.
///
/// [`Erase`]: super::Erase
#[derive(Debug, Clone, Copy, Default)]
pub struct DropRow;

#[async_trait::async_trait]
impl Operator<Tabular> for DropRow {
    fn id(&self) -> OperatorId {
        OperatorId::new("drop_row", "1.0.0")
    }

    fn leak_profile(&self) -> LeakProfile {
        // The record is gone entirely; no value or shape remains for it.
        LeakProfile::Irrecoverable
    }

    async fn anonymize(
        &self,
        _entity: &Entity<Tabular>,
        _data: &TextData,
    ) -> Result<TabularReplacement> {
        Ok(TabularReplacement::DropRow)
    }
}

#[cfg(test)]
mod tests {
    use elide_core::entity::provenance::{Event, PatternEvent, Provenance};
    use elide_core::entity::{Entity, LabelRef};
    use elide_core::modality::tabular::TabularLocation;
    use elide_core::primitive::Confidence;

    use super::*;
    use crate::operators::DropColumn;

    fn entity() -> Entity<Tabular> {
        let location = TabularLocation::new(1, 0);
        let event = Event::pattern(
            "t",
            Confidence::MAX,
            location.clone(),
            PatternEvent::default(),
        );
        Entity::new(
            LabelRef::new("PERSON"),
            location,
            Confidence::MAX,
            Provenance::new(event),
        )
    }

    #[tokio::test]
    async fn drop_operators_emit_their_structural_variant() {
        let data = TextData::new("Bob");
        assert_eq!(
            DropRow.anonymize(&entity(), &data).await.unwrap(),
            TabularReplacement::DropRow
        );
        assert_eq!(
            DropColumn.anonymize(&entity(), &data).await.unwrap(),
            TabularReplacement::DropColumn
        );
    }
}
