//! [`Text`] modality: plain or structured text addressed by byte ranges.

mod data;
mod location;
mod replacement;

pub use self::data::TextData;
pub use self::location::TextLocation;
pub use self::replacement::TextReplacement;
use super::Modality;

/// Text modality: data is [`TextData`], locations are
/// [`TextLocation`] byte ranges, replacements are [`TextReplacement`].
#[derive(Debug, Clone, Copy)]
pub struct Text;

impl Modality for Text {
    type Data = TextData;
    type Location = TextLocation;
    type Replacement = TextReplacement;

    const NAME: &'static str = "text";
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::redaction::Redactions;

    #[test]
    fn sort_by_position_orders_in_place() {
        let mut batch: Redactions<Text> = Redactions::new();
        // Pushed out of order.
        batch.push(TextLocation::new(20, 25), TextReplacement::Removed);
        batch.push(TextLocation::new(0, 5), TextReplacement::Removed);
        batch.push(TextLocation::new(10, 15), TextReplacement::Removed);

        batch.sort_by_position();

        let starts: Vec<usize> = batch.iter().map(|(loc, _)| loc.start).collect();
        assert_eq!(starts, [0, 10, 20]);
    }
}
