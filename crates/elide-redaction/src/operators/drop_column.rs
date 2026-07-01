//! [`DropColumn`]: drop the entire column a matched tabular entity sits in.

use elide_core::Result;
use elide_core::entity::Entity;
use elide_core::modality::tabular::{Tabular, TabularReplacement};
use elide_core::modality::text::TextData;
use elide_core::operator::{LeakProfile, Operator, OperatorId};

/// Drop the entire column a matched entity sits in, header included.
///
/// A structural treatment, coarser than [`DropRow`]: a single match drops
/// the whole column across every row. Useful for "this column is
/// identifying — remove it" (e.g. an `SSN` column). Bind it to the column's
/// label or to a predicate over the column name; any match in the column
/// removes it for all records.
///
/// [`DropRow`]: super::DropRow
#[derive(Debug, Clone, Copy, Default)]
pub struct DropColumn;

#[async_trait::async_trait]
impl Operator<Tabular> for DropColumn {
    fn id(&self) -> OperatorId {
        OperatorId::new("drop_column", "1.0.0")
    }

    fn leak_profile(&self) -> LeakProfile {
        // The column is gone entirely across all rows; nothing remains.
        LeakProfile::Irrecoverable
    }

    async fn anonymize(
        &self,
        _entity: &Entity<Tabular>,
        _data: &TextData,
    ) -> Result<TabularReplacement> {
        Ok(TabularReplacement::DropColumn)
    }
}
