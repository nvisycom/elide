//! Content streams, re-encoded through the PDF operator set.
//!
//! Every operator the specification defines for drawing is kept, and every
//! resource it names is copied into the rebuilt resources. What is cut:
//!
//! - the content of a layer hidden by default: each marked-content sequence it
//!   tags, and each XObject that belongs to it;
//! - marked content's property lists, which carry the alternate text assistive
//!   technology reads in place of the glyphs (`BDC` becomes `BMC`, `DP`
//!   becomes `MP`);
//! - inline images, and image XObjects no codec reads, removed rather than
//!   shipped unscanned;
//! - compatibility sections' markers and any operator the specification does
//!   not define.
//!
//! The result is balanced: a `Q` or `ET` with nothing open is cut, and what
//! is left open (a save, a text object, a marked-content sequence) is closed
//! at the end, so the content leaves the graphics state as it found it.

use elide_core::redaction::HiddenContent;
use elide_core::{Error, Result};
use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Object};

use super::keys::Kind;
use super::{Builder, MAX_NESTING};
use crate::pdf::reader::{DictionaryExt, DocumentExt, Scope};

/// Every content operator the PDF specification defines, but the inline image
/// and marked-content operators the rebuild handles itself.
const OPERATORS: &[&str] = &[
    "w", "J", "j", "M", "d", "ri", "i", "gs", "q", "Q", "cm", "m", "l", "c", "v", "y", "h", "re",
    "S", "s", "f", "F", "f*", "B", "B*", "b", "b*", "n", "W", "W*", "BT", "ET", "Tc", "Tw", "Tz",
    "TL", "Tf", "Tr", "Ts", "Td", "TD", "Tm", "T*", "Tj", "TJ", "'", "\"", "d0", "d1", "CS", "cs",
    "SC", "SCN", "sc", "scn", "G", "g", "RG", "rg", "K", "k", "sh", "Do",
];

/// The colour space families a `cs`/`CS` operand names directly (8.6.8).
const FAMILIES: &[&[u8]] = &[b"DeviceGray", b"DeviceRGB", b"DeviceCMYK", b"Pattern"];

/// The colour space resources that remap a device space (8.6.5.6).
const DEFAULT_SPACES: [&[u8]; 3] = [b"DefaultGray", b"DefaultRGB", b"DefaultCMYK"];

/// The prefix of a font resource name the rebuild gives a font a graphics
/// state selects.
const STATE_FONT_PREFIX: &str = "GSFont";

/// The property-list entries that carry text of their own: the replacement
/// text, an alternate description, and an expansion.
const ALTERNATE_TEXT_KEYS: [&[u8]; 3] = [b"ActualText", b"Alt", b"E"];

impl<'a> Builder<'a> {
    /// The content `bytes` rebuilt, with names resolved in `scope`, and the
    /// resources it names copied into `resources`.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if
    /// the content cannot be decoded or encoded, or as [`copy`](Self::copy).
    pub(super) fn content<'s>(
        &mut self,
        bytes: &[u8],
        scope: &Scope<'s>,
        resources: &mut Dictionary,
        depth: usize,
    ) -> Result<Vec<u8>>
    where
        'a: 's,
    {
        if depth > MAX_NESTING {
            return Err(Error::resource_limit(format!(
                "content nests deeper than {MAX_NESTING} levels"
            )));
        }
        let content =
            Content::decode(bytes).map_err(|e| Error::malformed(format!("decode content: {e}")))?;
        let mut kept: Vec<Operation> = Vec::with_capacity(content.operations.len());
        // One entry per open marked-content sequence: whether it hides.
        let mut open: Vec<bool> = Vec::new();
        // The open `q` saves and whether a text object is open, so the
        // rebuilt content is balanced and ends in the state it began in.
        let mut saved = 0usize;
        let mut in_text = false;
        // How many of the open marked-content sequences began inside the open
        // text object, so they are closed inside it.
        let mut marked_in_text = 0usize;
        for operation in content.operations {
            let operator = operation.operator.as_str();
            if open.contains(&true) {
                match operator {
                    "BDC" | "BMC" => open.push(false),
                    "EMC" => {
                        open.pop();
                    }
                    _ => {}
                }
                continue;
            }
            match operator {
                "BDC" | "DP" => {
                    let [Object::Name(tag), properties] = operation.operands.as_slice() else {
                        self.residue += 1;
                        if operator == "BDC" {
                            open.push(false);
                            marked_in_text += usize::from(in_text);
                            kept.push(Operation::new("BMC", vec![Object::Name(b"Span".to_vec())]));
                        }
                        continue;
                    };
                    let properties = match properties {
                        Object::Name(name) => scope.lookup(self.source, b"Properties", name),
                        other => Some(other),
                    };
                    if operator == "BDC"
                        && tag == b"OC"
                        && properties.is_some_and(|p| self.layers.is_hidden(p, 0))
                    {
                        self.record(HiddenContent::Layer);
                        open.push(true);
                        continue;
                    }
                    if properties
                        .and_then(|p| self.source.resolve_dict(p))
                        .is_some_and(|p| ALTERNATE_TEXT_KEYS.iter().any(|k| p.get(k).is_ok()))
                    {
                        self.record(HiddenContent::AlternativeText);
                    }
                    let tag = Object::Name(tag.clone());
                    if operator == "BDC" {
                        open.push(false);
                        marked_in_text += usize::from(in_text);
                        kept.push(Operation::new("BMC", vec![tag]));
                    } else {
                        kept.push(Operation::new("MP", vec![tag]));
                    }
                }
                "BMC" => {
                    open.push(false);
                    marked_in_text += usize::from(in_text);
                    kept.push(operation);
                }
                "EMC" => {
                    if open.pop().is_some() {
                        marked_in_text = marked_in_text.saturating_sub(usize::from(in_text));
                        kept.push(operation);
                    } else {
                        self.residue += 1;
                    }
                }
                "MP" => kept.push(operation),
                "BI" => self.unreadable(None),
                "Tf" => self.resource(
                    operation,
                    b"Font",
                    Kind::Font,
                    scope,
                    resources,
                    &mut kept,
                    depth,
                )?,
                "gs" => {
                    let font = self.state_font(&operation, scope, resources, depth)?;
                    self.resource(
                        operation,
                        b"ExtGState",
                        Kind::ExtGState,
                        scope,
                        resources,
                        &mut kept,
                        depth,
                    )?;
                    kept.extend(font);
                }
                "sh" => self.resource(
                    operation,
                    b"Shading",
                    Kind::Shading,
                    scope,
                    resources,
                    &mut kept,
                    depth,
                )?,
                "cs" | "CS" => match operation.operands.first() {
                    Some(Object::Name(name)) if FAMILIES.contains(&name.as_slice()) => {
                        kept.push(operation);
                    }
                    _ => self.resource(
                        operation,
                        b"ColorSpace",
                        Kind::ColorSpace,
                        scope,
                        resources,
                        &mut kept,
                        depth,
                    )?,
                },
                "scn" | "SCN" => match operation.operands.last() {
                    Some(Object::Name(_)) => self.resource(
                        operation,
                        b"Pattern",
                        Kind::Pattern,
                        scope,
                        resources,
                        &mut kept,
                        depth,
                    )?,
                    _ => kept.push(operation),
                },
                "Do" => self.draw(operation, scope, resources, &mut kept, depth)?,
                "BX" | "EX" => {}
                "q" => {
                    saved += 1;
                    kept.push(operation);
                }
                "Q" if saved == 0 => self.residue += 1,
                "Q" => {
                    saved -= 1;
                    kept.push(operation);
                }
                "BT" if in_text => self.residue += 1,
                "BT" => {
                    in_text = true;
                    kept.push(operation);
                }
                "ET" if !in_text => self.residue += 1,
                "ET" => {
                    // A sequence still open inside the text object closes first.
                    for _ in 0..marked_in_text {
                        open.pop();
                        kept.push(Operation::new("EMC", Vec::new()));
                    }
                    marked_in_text = 0;
                    in_text = false;
                    kept.push(operation);
                }
                _ if OPERATORS.contains(&operator) => kept.push(operation),
                _ => self.residue += 1,
            }
        }
        if in_text {
            for _ in 0..marked_in_text {
                open.pop();
                kept.push(Operation::new("EMC", Vec::new()));
            }
            kept.push(Operation::new("ET", Vec::new()));
        }
        for _ in open {
            kept.push(Operation::new("EMC", Vec::new()));
        }
        for _ in 0..saved {
            kept.push(Operation::new("Q", Vec::new()));
        }
        // A default colour space remaps the device space of that name for
        // everything this content draws in it.
        for name in DEFAULT_SPACES {
            if !has(resources, b"ColorSpace", name)
                && let Some(space) = scope.lookup(self.source, b"ColorSpace", name)
                && let Some(space) = self.copy(Kind::ColorSpace, space, scope, depth + 1)?
            {
                insert(resources, b"ColorSpace", name, space);
            }
        }
        let encoded = Content { operations: kept }
            .encode()
            .map_err(|e| Error::malformed(format!("encode content: {e}")))?;
        Ok(encoded)
    }

    /// Keep `operation`, which names a `category` resource by its first name
    /// operand (its last, for a pattern), copying the resource as `kind` into
    /// `resources`; drop it when the name resolves to nothing kept.
    #[allow(clippy::too_many_arguments)]
    fn resource<'s>(
        &mut self,
        operation: Operation,
        category: &[u8],
        kind: Kind,
        scope: &Scope<'s>,
        resources: &mut Dictionary,
        kept: &mut Vec<Operation>,
        depth: usize,
    ) -> Result<()>
    where
        'a: 's,
    {
        let name = match kind {
            Kind::Pattern => operation.operands.last(),
            _ => operation.operands.first(),
        };
        let Some(Object::Name(name)) = name else {
            self.residue += 1;
            return Ok(());
        };
        if !has(resources, category, name) {
            let Some(value) = scope.lookup(self.source, category, name) else {
                self.residue += 1;
                return Ok(());
            };
            let Some(copy) = self.copy(kind, value, scope, depth + 1)? else {
                return Ok(());
            };
            insert(resources, category, name, copy);
        }
        kept.push(operation);
        Ok(())
    }

    /// Keep the `Do` `operation` when its XObject is kept: an image a codec
    /// reads, or a form, neither in a hidden layer.
    fn draw<'s>(
        &mut self,
        operation: Operation,
        scope: &Scope<'s>,
        resources: &mut Dictionary,
        kept: &mut Vec<Operation>,
        depth: usize,
    ) -> Result<()>
    where
        'a: 's,
    {
        let source = self.source;
        let Some(Object::Name(name)) = operation.operands.first() else {
            self.residue += 1;
            return Ok(());
        };
        if has(resources, b"XObject", name) {
            kept.push(operation);
            return Ok(());
        }
        let Some(value) = scope.lookup(self.source, b"XObject", name) else {
            self.residue += 1;
            return Ok(());
        };
        let Some(Object::Stream(xobject)) = source.resolve(value) else {
            self.residue += 1;
            return Ok(());
        };
        let id = value.as_reference().ok();
        if xobject
            .dict
            .get(b"OC")
            .is_ok_and(|membership| self.layers.is_hidden(membership, 0))
        {
            self.record(HiddenContent::Layer);
            self.cut.extend(id);
            return Ok(());
        }
        let kind = match xobject.dict.name(self.source, b"Subtype") {
            Some(b"Form") => Kind::Form,
            Some(b"Image") => Kind::Image,
            Some(b"PS") => {
                self.record(HiddenContent::Script);
                self.cut.extend(id);
                return Ok(());
            }
            _ => {
                self.unreadable(id);
                return Ok(());
            }
        };
        if let Some(copy) = self.copy(kind, value, scope, depth + 1)? {
            insert(resources, b"XObject", name, copy);
            kept.push(operation);
        }
        Ok(())
    }

    /// The `Tf` that selects the font the graphics state `gs` names sets
    /// (`/Font [font size]`), its font copied into `resources` under a name of
    /// its own; `None` when the state sets no font. A graphics state's font is
    /// the same as a `Tf`, so writing it as one gives the text walk a single
    /// way fonts are selected.
    fn state_font<'s>(
        &mut self,
        gs: &Operation,
        scope: &Scope<'s>,
        resources: &mut Dictionary,
        depth: usize,
    ) -> Result<Option<Operation>>
    where
        'a: 's,
    {
        let source = self.source;
        let Some(Object::Name(name)) = gs.operands.first() else {
            return Ok(None);
        };
        let setting = scope
            .lookup(source, b"ExtGState", name)
            .and_then(|state| source.resolve_dict(state))
            .and_then(|state| state.get(b"Font").ok())
            .and_then(|font| source.resolve_array(font));
        let Some([font, size]) = setting.map(Vec::as_slice) else {
            return Ok(None);
        };
        let Some(size @ (Object::Integer(_) | Object::Real(_))) = source.resolve(size) else {
            self.residue += 1;
            return Ok(None);
        };
        let Some(font) = self.copy(Kind::Font, font, scope, depth + 1)? else {
            return Ok(None);
        };
        // The font keeps the name it already has here, else takes the first
        // name nothing in scope uses.
        let taken = |name: &[u8]| {
            has(resources, b"Font", name) || scope.lookup(self.source, b"Font", name).is_some()
        };
        let name = resources
            .get(b"Font")
            .and_then(Object::as_dict)
            .ok()
            .and_then(|fonts| fonts.iter().find(|(_, value)| **value == font))
            .map(|(name, _)| name.clone())
            .unwrap_or_else(|| {
                (1..)
                    .map(|index| format!("{STATE_FONT_PREFIX}{index}").into_bytes())
                    .find(|candidate| !taken(candidate))
                    .unwrap_or_default()
            });
        insert(resources, b"Font", &name, font);
        Ok(Some(Operation::new(
            "Tf",
            vec![Object::Name(name), size.clone()],
        )))
    }
}

/// Whether `resources` already holds `name` under `category`.
pub(super) fn has(resources: &Dictionary, category: &[u8], name: &[u8]) -> bool {
    resources
        .get(category)
        .and_then(Object::as_dict)
        .is_ok_and(|entries| entries.get(name).is_ok())
}

/// Set `name` to `value` under `category` of `resources`.
pub(super) fn insert(resources: &mut Dictionary, category: &[u8], name: &[u8], value: Object) {
    if resources.get(category).is_err() {
        resources.set(category.to_vec(), Dictionary::new());
    }
    if let Ok(Object::Dictionary(entries)) = resources.get_mut(category) {
        entries.set(name.to_vec(), value);
    }
}
