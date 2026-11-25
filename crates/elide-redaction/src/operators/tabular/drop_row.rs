//! [`DropRow`]: drop the entire row a matched tabular entity sits in.

use elide_core::entity::Entity;
use elide_core::modality::tabular::{Tabular, TabularReplacement};
use elide_core::modality::text::TextData;
use elide_core::redaction::{LeakProfile, Operator};
use elide_core::{Component, Result};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Drops the entire row an entity sits in, rather than editing one cell.
///
/// A structural treatment, not a cell edit: where [`Remove`] blanks one
/// cell's text, [`DropRow`] removes the whole record. Useful for "this row
/// names a sanctioned individual, drop it entirely". Any match in a row
/// drops that row, so the table shrinks by one record per matched row; a
/// match in the header row drops the header.
///
/// [`Remove`]: crate::operators::Remove
#[derive(Debug, Clone, Copy, Default, Component)]
#[component(name = "drop_row", version = "1.0.0")]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct DropRow;

#[async_trait::async_trait]
impl Operator<Tabular> for DropRow {
    fn leak_profile(&self) -> LeakProfile {
        // The record is gone entirely; no value or shape remains for it.
        LeakProfile::Irrecoverable
    }

    async fn anonymize(
        &self,
        _entity: &Entity<Tabular>,
        _data: &TextData,
    ) -> Result<Option<TabularReplacement>> {
        Ok(Some(TabularReplacement::RowDropped))
    }
}

#[cfg(test)]
mod tests {
    use elide_core::entity::audit::{Pattern, PatternRule};
    use elide_core::entity::{Birth, Entity, LabelRef};
    use elide_core::modality::tabular::TabularLocation;
    use elide_core::primitive::Confidence;

    use super::*;
    use crate::operators::DropColumn;

    fn entity() -> Entity<Tabular> {
        let location = TabularLocation::new(1, 0);
        Entity::new(Birth::new(
            "t",
            Confidence::MAX,
            Pattern::new(LabelRef::new("PERSON"), location, PatternRule::default()),
        ))
    }

    #[tokio::test]
    async fn drop_operators_emit_their_structural_variant() {
        let data = TextData::new("Bob");
        assert_eq!(
            DropRow.anonymize(&entity(), &data).await.unwrap(),
            Some(TabularReplacement::RowDropped)
        );
        assert_eq!(
            DropColumn.anonymize(&entity(), &data).await.unwrap(),
            Some(TabularReplacement::ColumnDropped)
        );
    }
}
