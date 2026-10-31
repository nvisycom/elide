//! The names a workbook holds to Excel's rules, so a workbook whose names are
//! redacted still opens (see [`opc::names`](crate::opc::names)).
//!
//! - A sheet name is 1 to 31 characters, none of `\ / ? * [ ] :`, does not
//!   start or end with an apostrophe, and is unique in the workbook.
//! - A defined name and a table's display name are identifiers sharing one
//!   namespace: letters, digits, `_`, `.`, and `\`, starting with a letter,
//!   `_`, or `\`, and not shaped like a cell reference. A table's own name is
//!   an identifier unique among tables.
//! - A table column's name is non-empty and unique in its table.
//!
//! A hyperlink's location, a defined name or a sheet-qualified cell, follows
//! a renamed name. A formula naming one does not.

use super::sanitize::{
    DEFINED_NAME, HYPERLINK_LOCATION, SHEET_NAME, TABLE_COLUMN_NAME, TABLE_DISPLAY_NAME, TABLE_NAME,
};
use crate::opc::Names;
use crate::opc::names::{NameReference, NameRule, Numbering, ReferenceForm};

/// The longest sheet name Excel accepts, in characters.
const MAX_SHEET_NAME: usize = 31;
/// The longest defined or table name Excel accepts, in characters.
const MAX_IDENTIFIER: usize = 255;

/// The names of a workbook.
pub(super) static NAMES: Names = Names {
    rules: &[
        NameRule {
            attribute: SHEET_NAME,
            group: "sheet",
            per_part: false,
            fit: fit_sheet_name,
            max: Some(MAX_SHEET_NAME),
            numbering: Numbering::Parenthesized,
        },
        NameRule {
            attribute: DEFINED_NAME,
            group: "name",
            per_part: false,
            fit: fit_identifier,
            max: Some(MAX_IDENTIFIER),
            numbering: Numbering::Underscored,
        },
        NameRule {
            attribute: TABLE_DISPLAY_NAME,
            group: "name",
            per_part: false,
            fit: fit_identifier,
            max: Some(MAX_IDENTIFIER),
            numbering: Numbering::Underscored,
        },
        NameRule {
            attribute: TABLE_NAME,
            group: "table",
            per_part: false,
            fit: fit_identifier,
            max: Some(MAX_IDENTIFIER),
            numbering: Numbering::Underscored,
        },
        NameRule {
            attribute: TABLE_COLUMN_NAME,
            group: "column",
            per_part: true,
            fit: fit_column_name,
            max: None,
            numbering: Numbering::Parenthesized,
        },
    ],
    references: &[
        NameReference {
            attribute: HYPERLINK_LOCATION,
            group: "sheet",
            form: ReferenceForm::SheetQualified,
        },
        NameReference {
            attribute: HYPERLINK_LOCATION,
            group: "name",
            form: ReferenceForm::Exact,
        },
    ],
};

/// `name` as a sheet name: brackets become parentheses, the other forbidden
/// characters underscores, surrounding apostrophes are dropped, and the result
/// is capped at [`MAX_SHEET_NAME`] characters; an empty result is `Sheet`.
fn fit_sheet_name(name: &str) -> String {
    let fitted: String = name
        .chars()
        .map(|c| match c {
            '[' => '(',
            ']' => ')',
            '\\' | '/' | '?' | '*' | ':' => '_',
            c => c,
        })
        .collect::<String>()
        .trim_matches('\'')
        .chars()
        .take(MAX_SHEET_NAME)
        .collect();
    if fitted.trim().is_empty() {
        "Sheet".to_owned()
    } else {
        fitted
    }
}

/// `name` as a defined or table name: every character but a letter, a digit,
/// `_`, `.`, and `\` becomes `_`, and one that starts with anything but a
/// letter, `_`, or `\`, or reads as a cell reference, gets a leading `_`; an
/// empty result is `Name`.
fn fit_identifier(name: &str) -> String {
    let fitted: String = name
        .chars()
        .map(|c| match c {
            c if c.is_alphanumeric() || matches!(c, '_' | '.' | '\\') => c,
            _ => '_',
        })
        .take(MAX_IDENTIFIER)
        .collect();
    let Some(first) = fitted.chars().next() else {
        return "Name".to_owned();
    };
    let starts_well = first.is_alphabetic() || matches!(first, '_' | '\\');
    if starts_well && !is_cell_reference(&fitted) {
        fitted
    } else {
        format!("_{fitted}").chars().take(MAX_IDENTIFIER).collect()
    }
}

/// Whether `name` reads as a cell reference (`A1`, `XFD9`, `R1C1`, `R`, `C`).
fn is_cell_reference(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    let letters = upper.chars().take_while(char::is_ascii_alphabetic).count();
    let digits = &upper[letters..];
    let a1 = (1..=3).contains(&letters)
        && !digits.is_empty()
        && digits.chars().all(|c| c.is_ascii_digit());
    let r1c1 = upper
        .strip_prefix('R')
        .map(|rest| rest.trim_start_matches(|c: char| c.is_ascii_digit()))
        .and_then(|rest| rest.strip_prefix('C'))
        .is_some_and(|rest| rest.chars().all(|c| c.is_ascii_digit()));
    a1 || r1c1 || upper == "R" || upper == "C"
}

/// `name` as a table column's name; an empty result is `Column`.
fn fit_column_name(name: &str) -> String {
    if name.trim().is_empty() {
        "Column".to_owned()
    } else {
        name.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sheet_name_loses_its_forbidden_characters() {
        assert_eq!(fit_sheet_name("[PERSON]"), "(PERSON)");
        assert_eq!(fit_sheet_name("a/b:c"), "a_b_c");
        assert_eq!(fit_sheet_name("'x'"), "x");
        assert_eq!(fit_sheet_name(""), "Sheet");
        assert_eq!(fit_sheet_name(&"x".repeat(40)).chars().count(), 31);
    }

    #[test]
    fn an_identifier_is_fitted_to_excel_names() {
        assert_eq!(fit_identifier("[PERSON]_Salary"), "_PERSON__Salary");
        assert_eq!(fit_identifier("Jane Roe"), "Jane_Roe");
        assert_eq!(fit_identifier("2024 budget"), "_2024_budget");
        assert_eq!(fit_identifier("AB12"), "_AB12");
        assert_eq!(fit_identifier("R2C3"), "_R2C3");
        assert_eq!(fit_identifier(""), "Name");
    }
}
