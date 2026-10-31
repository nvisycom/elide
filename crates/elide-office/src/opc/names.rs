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

use elide_core::{Error, ErrorKind, Result};
use quick_xml::escape::unescape;

use crate::opc::attributes::TextAttribute;
use crate::opc::block::Replacement;
use crate::opc::part::PartPath;
use crate::opc::xml_span::attribute_spans;

/// A format's names and the attributes referring to them.
#[derive(Debug, Clone, Copy)]
pub struct Names {
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

/// One kind of name and the rules a redacted one is fitted to.
#[derive(Debug, Clone, Copy)]
pub struct NameRule {
    /// The attribute holding the name.
    pub(crate) attribute: TextAttribute,
    /// The group the name is unique in. Names of different rules in one group
    /// are unique together.
    pub(crate) group: &'static str,
    /// Whether the name is unique in its part, rather than in the package.
    pub(crate) per_part: bool,
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
pub struct NameReference {
    /// The attribute holding the reference. It is a text attribute too, so it
    /// can be spliced.
    pub(crate) attribute: TextAttribute,
    pub(crate) group: &'static str,
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

/// `replacements` with each one landing in a name folded into a replacement of
/// the whole name, fitted to its rule and unique in its group, and every
/// reference to a renamed name rewritten. `parts` are the package's XML text
/// parts and their text.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`] if a part is not well-formed XML or holds a
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

    // Every name, with the redaction that lands in it.
    let mut found: Vec<Found> = Vec::new();
    for (part, raw) in parts.clone() {
        for rule in names.rules {
            for range in spans(part, raw, rule.attribute)? {
                let (inside, outside) = take_inside(pending, &range);
                pending = outside;
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

    // Fit each redacted name, unique among its group's final names.
    let group_of = |name: &Found| {
        let part = name.rule.per_part.then(|| name.part.clone());
        (name.rule.group, part)
    };
    let mut taken: HashMap<(&str, Option<PartPath>), HashSet<String>> = HashMap::new();
    for name in found.iter().filter(|name| name.redacted.is_none()) {
        taken
            .entry(group_of(name))
            .or_default()
            .insert(name.original.to_lowercase());
    }
    let mut out: Vec<Replacement> = Vec::new();
    // A renamed name's group and old (lowercased) value, to its new value.
    let mut renamed: HashMap<(&str, String), String> = HashMap::new();
    for name in &found {
        let Some(redacted) = &name.redacted else {
            continue;
        };
        let taken = taken.entry(group_of(name)).or_default();
        let fitted = unique((name.rule.fit)(redacted), name.rule, taken);
        taken.insert(fitted.to_lowercase());
        renamed.insert(
            (name.rule.group, name.original.to_lowercase()),
            fitted.clone(),
        );
        out.push(Replacement {
            part: name.part.clone(),
            start: name.range.start,
            end: name.range.end,
            text: fitted.into(),
        });
    }

    // Point every reference to a renamed name at its new one. An attribute
    // that may refer to names of several groups is retargeted once.
    if !renamed.is_empty() {
        let mut retargeted: HashSet<(PartPath, usize)> = HashSet::new();
        for (part, raw) in parts {
            for reference in names.references {
                for range in spans(part, raw, reference.attribute)? {
                    if retargeted.contains(&(part.clone(), range.start)) {
                        continue;
                    }
                    let value = decoded(part, &raw[range.clone()])?;
                    let Some(text) = retarget(&value, reference, &renamed) else {
                        continue;
                    };
                    let (_, outside) = take_inside(pending, &range);
                    pending = outside;
                    retargeted.insert((part.clone(), range.start));
                    out.push(Replacement {
                        part: part.clone(),
                        start: range.start,
                        end: range.end,
                        text: text.into(),
                    });
                }
            }
        }
    }

    out.extend(pending.into_iter().cloned());
    Ok(out)
}

/// The value ranges of `attribute` in `raw`.
fn spans(part: &PartPath, raw: &str, attribute: TextAttribute) -> Result<Vec<Range<usize>>> {
    attribute_spans(raw, &[attribute])
        .map(|spans| spans.into_iter().map(|span| span.range).collect())
        .map_err(|()| Error::new(ErrorKind::MalformedInput, format!("`{part}` malformed XML")))
}

/// `pending` split into the replacements inside `range` and the rest.
fn take_inside<'r>(
    pending: Vec<&'r Replacement>,
    range: &Range<usize>,
) -> (Vec<&'r Replacement>, Vec<&'r Replacement>) {
    pending
        .into_iter()
        .partition(|r| range.start <= r.start && r.end <= range.end)
}

/// The value at `range` of `raw` with `inside` applied, entities resolved.
fn redacted(
    part: &PartPath,
    raw: &str,
    range: &Range<usize>,
    inside: &[&Replacement],
) -> Result<String> {
    let mut inside = inside.to_vec();
    inside.sort_by_key(|r| r.start);
    let mut value = String::new();
    let mut cursor = range.start;
    for r in inside {
        value.push_str(&decoded(part, &raw[cursor..r.start])?);
        value.push_str(&r.text);
        cursor = r.end;
    }
    value.push_str(&decoded(part, &raw[cursor..range.end])?);
    Ok(value)
}

/// `raw` attribute text with its entities resolved.
fn decoded(part: &PartPath, raw: &str) -> Result<String> {
    unescape(raw).map(|text| text.into_owned()).map_err(|e| {
        Error::new(
            ErrorKind::MalformedInput,
            format!("`{part}` has a malformed attribute value: {e}"),
        )
    })
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
fn retarget(
    value: &str,
    reference: &NameReference,
    renamed: &HashMap<(&str, String), String>,
) -> Option<String> {
    let new = |old: &str| renamed.get(&(reference.group, old.to_lowercase()));
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
    use crate::opc::attributes::Ns;
    use crate::opc::xml_edit::Local;

    const SHEET: TextAttribute =
        TextAttribute::new((Ns::Any, Local::Exact("sheet")), (Ns::None, "name"));
    const LOCATION: TextAttribute =
        TextAttribute::new((Ns::Any, Local::Exact("hyperlink")), (Ns::None, "location"));

    static NAMES: Names = Names {
        rules: &[NameRule {
            attribute: SHEET,
            group: "sheet",
            per_part: false,
            fit: |name| name.replace(['[', ']'], "_"),
            max: Some(31),
            numbering: Numbering::Parenthesized,
        }],
        references: &[NameReference {
            attribute: LOCATION,
            group: "sheet",
            form: ReferenceForm::SheetQualified,
        }],
    };

    fn at(part: &PartPath, raw: &str, value: &str) -> Replacement {
        let start = raw.find(value).unwrap();
        Replacement {
            part: part.clone(),
            start,
            end: start + value.len(),
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
