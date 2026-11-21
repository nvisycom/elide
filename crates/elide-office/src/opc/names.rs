//! [`Names`]: the names a format holds to rules of their own (a sheet name, a
//! bookmark, a defined name), and the attributes that refer to them, so a
//! redacted name still leaves a document that opens.
//!
//! A name is a [text attribute](crate::opc::TextAttribute): detection sees it
//! and may redact part of it. A redaction landing in a name is folded into one
//! replacement of the whole name, fitted to its [`NameRule`] (its character set
//! and length) and made unique in its group regardless of case. Every
//! [`NameReference`] to a renamed name is rewritten to the new one, so a
//! hyperlink to a bookmark or a sheet keeps working. A reference inside a
//! formula or a field code is not rewritten.

use std::collections::{HashMap, HashSet};
use std::ops::Range;

use elide_core::{Error, Result};

use crate::opc::block::Replacement;
use crate::opc::package::part::PartPath;
use crate::opc::xml::attributes::TextAttribute;
use crate::opc::xml::span::attribute_ranges;
use crate::opc::{attribute_value, malformed_part};

/// A format's names and the attributes referring to them.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Names {
    pub(crate) rules: &'static [NameRule],
    pub(crate) references: &'static [NameReference],
}

impl Names {
    /// No names.
    pub(crate) const NONE: Self = Self {
        rules: &[],
        references: &[],
    };
}

/// The group a name is unique in, and a reference refers into, such as
/// `sheet` or `bookmark`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct NameGroup(&'static str);

impl NameGroup {
    /// The group called `name`.
    pub(crate) const fn new(name: &'static str) -> Self {
        Self(name)
    }
}

/// Where a name is unique.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Unique {
    /// In the whole package.
    InPackage,
    /// In its own part.
    InPart,
}

/// One kind of name and the rules a redacted one is fitted to.
#[derive(Debug, Clone, Copy)]
pub(crate) struct NameRule {
    /// The attribute holding the name.
    pub(crate) attribute: TextAttribute,
    /// The group the name is unique in. Names of different rules in one group
    /// are unique together.
    pub(crate) group: NameGroup,
    /// Where the name is unique in its group.
    pub(crate) unique: Unique,
    /// The name with its forbidden characters replaced; never empty.
    pub(crate) fit: fn(&str) -> String,
    /// The longest name, in characters, or `None` when unbounded.
    pub(crate) max: Option<usize>,
    /// How a taken name is numbered to make it unique.
    pub(crate) numbering: Numbering,
}

/// How a taken name is numbered.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Numbering {
    /// `name (2)`, for a name that may hold spaces and parentheses.
    Parenthesized,
    /// `name_2`, for an identifier.
    Underscored,
}

impl Numbering {
    fn suffix(self, n: usize) -> String {
        match self {
            Self::Parenthesized => format!(" ({n})"),
            Self::Underscored => format!("_{n}"),
        }
    }
}

/// An attribute that refers to a name of `group`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct NameReference {
    /// The attribute holding the reference. It is a text attribute too, so it
    /// can be spliced.
    pub(crate) attribute: TextAttribute,
    pub(crate) group: NameGroup,
    pub(crate) form: ReferenceForm,
}

/// How a reference names its target.
#[derive(Debug, Clone, Copy)]
pub(crate) enum ReferenceForm {
    /// The whole value is the name.
    Exact,
    /// The value is a sheet-qualified reference, `Sheet!A1` or `'A Sheet'!A1`.
    SheetQualified,
}

/// One name found in the package.
struct Found {
    part: PartPath,
    range: Range<usize>,
    rule: &'static NameRule,
    original: String,
    /// The redacted value, when a replacement lands in the name.
    redacted: Option<String>,
}

impl Found {
    /// Where the name must be unique.
    fn scope(&self) -> Scope {
        Scope {
            group: self.rule.group,
            part: match self.rule.unique {
                Unique::InPackage => None,
                Unique::InPart => Some(self.part.clone()),
            },
        }
    }
}

/// Where a name must be unique: its group, within its part when the name is
/// unique per part.
#[derive(PartialEq, Eq, Hash)]
struct Scope {
    group: NameGroup,
    part: Option<PartPath>,
}

/// A renamed name: its group and its old value, lowercased.
#[derive(PartialEq, Eq, Hash)]
struct OldName {
    group: NameGroup,
    lowercase: String,
}

/// Each renamed name's new value.
type Renamed = HashMap<OldName, String>;

/// `replacements` with each one landing in a name folded into a replacement of
/// the whole name, fitted to its rule and unique in its group, and every
/// reference to a renamed name rewritten. `parts` are the package's XML text
/// parts and their text.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if a part is not well-formed XML or holds a
/// malformed attribute value.
pub(crate) fn fit_names<'a>(
    names: &'static Names,
    parts: impl Iterator<Item = (&'a PartPath, &'a str)> + Clone,
    replacements: &[Replacement],
) -> Result<Vec<Replacement>> {
    if names.rules.is_empty() || replacements.is_empty() {
        return Ok(replacements.to_vec());
    }
    let mut pending: Vec<&Replacement> = replacements.iter().collect();
    let found = find_names(names, parts.clone(), &mut pending)?;
    let (mut out, renamed) = fit_found(&found);
    if !renamed.is_empty() {
        for (part, raw) in parts {
            out.extend(retarget_part(names, part, raw, &renamed, &mut pending)?);
        }
    }
    out.extend(pending.into_iter().cloned());
    Ok(out)
}

/// Every name in `parts`, with the redaction that lands in it taken from
/// `pending`.
fn find_names<'a>(
    names: &'static Names,
    parts: impl Iterator<Item = (&'a PartPath, &'a str)>,
    pending: &mut Vec<&Replacement>,
) -> Result<Vec<Found>> {
    let mut found: Vec<Found> = Vec::new();
    let rule_attributes: Vec<TextAttribute> = names.rules.iter().map(|r| r.attribute).collect();
    for (part, raw) in parts {
        let ranges = spans(part, raw, &rule_attributes)?;
        for (rule, ranges) in names.rules.iter().zip(ranges) {
            for range in ranges {
                let (inside, outside) = take_inside(std::mem::take(pending), part, &range);
                *pending = outside;
                found.push(Found {
                    part: part.clone(),
                    original: decoded(part, &raw[range.clone()])?,
                    redacted: (!inside.is_empty())
                        .then(|| redacted(part, raw, &range, &inside))
                        .transpose()?,
                    range,
                    rule,
                });
            }
        }
    }
    Ok(found)
}

/// A replacement of each redacted name in `found` fitted to its rule, unique
/// among its group's final names, and each renamed name's new value.
fn fit_found(found: &[Found]) -> (Vec<Replacement>, Renamed) {
    let mut taken: HashMap<Scope, HashSet<String>> = HashMap::new();
    for name in found.iter().filter(|name| name.redacted.is_none()) {
        taken
            .entry(name.scope())
            .or_default()
            .insert(name.original.to_lowercase());
    }
    let mut out: Vec<Replacement> = Vec::new();
    let mut renamed = Renamed::new();
    for name in found {
        let Some(redacted) = &name.redacted else {
            continue;
        };
        let taken = taken.entry(name.scope()).or_default();
        let fitted = unique((name.rule.fit)(redacted), name.rule, taken);
        taken.insert(fitted.to_lowercase());
        let old = OldName {
            group: name.rule.group,
            lowercase: name.original.to_lowercase(),
        };
        renamed.insert(old, fitted.clone());
        out.push(Replacement {
            part: name.part.clone(),
            span: name.range.clone(),
            text: fitted.into(),
        });
    }
    (out, renamed)
}

/// A replacement of every reference in `part` to a `renamed` name pointing
/// it at its new one, each taking the place of what `pending` had for it. An
/// attribute that may refer to names of several groups is retargeted once.
fn retarget_part(
    names: &'static Names,
    part: &PartPath,
    raw: &str,
    renamed: &Renamed,
    pending: &mut Vec<&Replacement>,
) -> Result<Vec<Replacement>> {
    let mut out = Vec::new();
    let mut retargeted: HashSet<usize> = HashSet::new();
    let reference_attributes: Vec<TextAttribute> =
        names.references.iter().map(|r| r.attribute).collect();
    let ranges = spans(part, raw, &reference_attributes)?;
    for (reference, ranges) in names.references.iter().zip(ranges) {
        for range in ranges {
            if retargeted.contains(&range.start) {
                continue;
            }
            let value = decoded(part, &raw[range.clone()])?;
            let Some(text) = retarget(&value, reference, renamed) else {
                continue;
            };
            let (_, outside) = take_inside(std::mem::take(pending), part, &range);
            *pending = outside;
            retargeted.insert(range.start);
            out.push(Replacement {
                part: part.clone(),
                span: range.clone(),
                text: text.into(),
            });
        }
    }
    Ok(out)
}

/// The value ranges in `raw` of each of `attributes`, one list per attribute,
/// read in one pass over the part.
fn spans(
    part: &PartPath,
    raw: &str,
    attributes: &[TextAttribute],
) -> Result<Vec<Vec<Range<usize>>>> {
    attribute_ranges(raw, attributes).map_err(|_| malformed_part(part))
}

/// `pending` split into the replacements inside `range` of `part` and the
/// rest. Another part's replacement at the same offsets stays pending.
fn take_inside<'r>(
    pending: Vec<&'r Replacement>,
    part: &PartPath,
    range: &Range<usize>,
) -> (Vec<&'r Replacement>, Vec<&'r Replacement>) {
    pending
        .into_iter()
        .partition(|r| &r.part == part && range.start <= r.span.start && r.span.end <= range.end)
}

/// The value at `range` of `raw` with `inside` applied, entities resolved.
fn redacted(
    part: &PartPath,
    raw: &str,
    range: &Range<usize>,
    inside: &[&Replacement],
) -> Result<String> {
    let mut inside = inside.to_vec();
    inside.sort_by_key(|r| r.span.start);
    let mut value = String::new();
    let mut cursor = range.start;
    for r in inside {
        value.push_str(&decoded(part, &raw[cursor..r.span.start])?);
        value.push_str(&r.text);
        cursor = r.span.end;
    }
    value.push_str(&decoded(part, &raw[cursor..range.end])?);
    Ok(value)
}

/// `raw` attribute text with its entities resolved.
fn decoded(part: &PartPath, raw: &str) -> Result<String> {
    attribute_value(raw)
        .map(|text| text.into_owned())
        .map_err(|e| Error::malformed(format!("`{part}` has a malformed attribute value: {e}")))
}

/// `name`, or it numbered by `rule` within its length, whichever is first not
/// in `taken` regardless of case.
fn unique(name: String, rule: &NameRule, taken: &HashSet<String>) -> String {
    if !taken.contains(&name.to_lowercase()) {
        return name;
    }
    (2..)
        .map(|n| {
            let suffix = rule.numbering.suffix(n);
            let base: String = match rule.max {
                Some(max) => name
                    .chars()
                    .take(max.saturating_sub(suffix.chars().count()))
                    .collect(),
                None => name.clone(),
            };
            base + &suffix
        })
        .find(|candidate| !taken.contains(&candidate.to_lowercase()))
        .expect("an unbounded sequence of names has an untaken one")
}

/// The reference `value` pointed at its target's new name, or `None` when its
/// target was not renamed.
fn retarget(value: &str, reference: &NameReference, renamed: &Renamed) -> Option<String> {
    let new = |old: &str| {
        renamed.get(&OldName {
            group: reference.group,
            lowercase: old.to_lowercase(),
        })
    };
    match reference.form {
        ReferenceForm::Exact => new(value).cloned(),
        ReferenceForm::SheetQualified => {
            let (sheet, rest) = value.rsplit_once('!')?;
            let sheet = match sheet.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')) {
                Some(quoted) => quoted.replace("''", "'"),
                None => sheet.to_owned(),
            };
            let new = new(&sheet)?;
            Some(format!("'{}'!{rest}", new.replace('\'', "''")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHEET: TextAttribute = TextAttribute::unqualified("sheet", "name");
    const LOCATION: TextAttribute = TextAttribute::unqualified("hyperlink", "location");

    const SHEETS: NameGroup = NameGroup::new("sheet");

    static NAMES: Names = Names {
        rules: &[NameRule {
            attribute: SHEET,
            group: SHEETS,
            unique: Unique::InPackage,
            fit: |name| name.replace(['[', ']'], "_"),
            max: Some(31),
            numbering: Numbering::Parenthesized,
        }],
        references: &[NameReference {
            attribute: LOCATION,
            group: SHEETS,
            form: ReferenceForm::SheetQualified,
        }],
    };

    fn at(part: &PartPath, raw: &str, value: &str) -> Replacement {
        let start = raw.find(value).unwrap();
        Replacement {
            part: part.clone(),
            span: start..start + value.len(),
            text: "[PERSON]".into(),
        }
    }

    #[test]
    fn names_redacted_alike_stay_distinct_and_references_follow() {
        let workbook = PartPath::from("xl/workbook.xml");
        let sheet = PartPath::from("xl/worksheets/sheet1.xml");
        let book = r#"<workbook><sheet name="Jane Roe"/><sheet name="John Doe"/></workbook>"#;
        let cells = r#"<worksheet><hyperlink ref="A1" location="'Jane Roe'!B2"/></worksheet>"#;
        let parts = [(&workbook, book), (&sheet, cells)];
        let fitted = fit_names(
            &NAMES,
            parts.iter().copied(),
            &[
                at(&workbook, book, "Jane Roe"),
                at(&workbook, book, "John Doe"),
            ],
        )
        .unwrap();
        let text = |part: &PartPath| -> Vec<String> {
            let mut texts: Vec<String> = fitted
                .iter()
                .filter(|r| &r.part == part)
                .map(|r| r.text.to_string())
                .collect();
            texts.sort_unstable();
            texts
        };
        assert_eq!(text(&workbook), ["_PERSON_", "_PERSON_ (2)"]);
        assert_eq!(text(&sheet), ["'_PERSON_'!B2"]);
    }

    /// A replacement lands in a name only in its own part, though another
    /// part holds a name at the same offsets.
    #[test]
    fn a_replacement_lands_only_in_its_own_part() {
        let first = PartPath::from("xl/workbook.xml");
        let second = PartPath::from("xl/workbook2.xml");
        let raw = r#"<workbook><sheet name="Jane Roe"/></workbook>"#;
        let parts = [(&first, raw), (&second, raw)];
        let fitted = fit_names(
            &NAMES,
            parts.iter().copied(),
            &[at(&second, raw, "Jane Roe")],
        )
        .unwrap();
        assert_eq!(fitted.len(), 1, "{fitted:?}");
        assert_eq!(fitted[0].part, second);
        assert_eq!(&*fitted[0].text, "_PERSON_");
    }

    #[test]
    fn a_numbered_name_keeps_within_its_length() {
        let rule = &NAMES.rules[0];
        let long = "y".repeat(31);
        let taken: HashSet<String> = [long.clone()].into();
        let fitted = unique(long, rule, &taken);
        assert_eq!(fitted.chars().count(), 31);
        assert!(fitted.ends_with(" (2)"));
    }
}
