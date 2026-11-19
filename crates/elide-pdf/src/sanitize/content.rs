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

use std::collections::BTreeMap;

use elide_core::redaction::HiddenContent;
use elide_core::{Error, Result};
use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Object, ObjectId};

use super::copy::At;
use super::keys::Kind;
use super::{Builder, MAX_NESTING};
use crate::pdf::reader::{DictionaryExt, DocumentExt};

/// Every content operator the PDF specification defines, but the inline image
/// and marked-content operators the rebuild handles itself.
const OPERATORS: &[&str] = &[
    "w", "J", "j", "M", "d", "ri", "i", "gs", "q", "Q", "cm", "m", "l", "c", "v", "y", "h", "re",
    "S", "s", "f", "F", "f*", "B", "B*", "b", "b*", "n", "W", "W*", "BT", "ET", "Tc", "Tw", "Tz",
    "TL", "Tf", "Tr", "Ts", "Td", "TD", "Tm", "T*", "Tj", "TJ", "'", "\"", "d0", "d1", "CS", "cs",
    "SC", "SCN", "sc", "scn", "G", "g", "RG", "rg", "K", "k", "sh", "Do",
];

/// An open marked-content sequence: one written (as `BMC`), one hidden by
/// its layer and cut, or one inside a hidden one, cut with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Marked {
    Written,
    Hiding,
    Within,
}

/// An open marked-content sequence, and whether it began inside a text
/// object, so it closes inside it.
#[derive(Debug, Clone, Copy)]
struct Open {
    marked: Marked,
    in_text: bool,
}

/// The colour space families a `cs`/`CS` operand names directly (8.6.8).
const FAMILIES: &[&[u8]] = &[b"DeviceGray", b"DeviceRGB", b"DeviceCMYK", b"Pattern"];

/// The colour space resources that remap a device space (8.6.5.6).
const DEFAULT_SPACES: [&[u8]; 3] = [b"DefaultGray", b"DefaultRGB", b"DefaultCMYK"];

/// The prefix of a font resource name the rebuild gives a font a graphics
/// state selects.
const STATE_FONT_PREFIX: &str = "GSFont";

/// The prefix of an XObject resource name the rebuild gives a second copy
/// of a form drawn under one name.
const DRAWN_PREFIX: &str = "Fx";

/// The property-list entries that carry text of their own: the replacement
/// text, an alternate description, and an expansion.
const ALTERNATE_TEXT_KEYS: [&[u8]; 3] = [b"ActualText", b"Alt", b"E"];

/// A resource content names by an operand: the category of resources it is
/// listed under, and the kind it is copied as.
#[derive(Debug, Clone, Copy)]
enum Resource {
    Font,
    ExtGState,
    Shading,
    ColorSpace,
    Pattern,
}

impl Resource {
    /// The category of resources it is listed under.
    fn category(self) -> &'static [u8] {
        match self {
            Self::Font => b"Font",
            Self::ExtGState => b"ExtGState",
            Self::Shading => b"Shading",
            Self::ColorSpace => b"ColorSpace",
            Self::Pattern => b"Pattern",
        }
    }

    /// The kind it is copied as.
    fn kind(self) -> Kind {
        match self {
            Self::Font => Kind::Font,
            Self::ExtGState => Kind::ExtGState,
            Self::Shading => Kind::Shading,
            Self::ColorSpace => Kind::ColorSpace,
            Self::Pattern => Kind::Pattern,
        }
    }

    /// The operand of `operation` that names it: a pattern's last, after any
    /// colour components; any other's first.
    fn name(self, operation: &Operation) -> Option<&Object> {
        match self {
            Self::Pattern => operation.operands.last(),
            _ => operation.operands.first(),
        }
    }
}

impl<'a> Builder<'a> {
    /// The content `bytes` rebuilt at `at`, with names resolved in its scope,
    /// and the resources it names copied into `resources`.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if
    /// the content cannot be decoded or encoded, or as [`copy`](Self::copy).
    pub(super) fn content<'s>(
        &mut self,
        bytes: &[u8],
        at: At<'_, 's>,
        resources: &mut Dictionary,
    ) -> Result<Vec<u8>>
    where
        'a: 's,
    {
        if at.depth > MAX_NESTING {
            return Err(Error::resource_limit(format!(
                "content nests deeper than {MAX_NESTING} levels"
            )));
        }
        let content =
            Content::decode(bytes).map_err(|e| Error::malformed(format!("decode content: {e}")))?;
        let mut copy = ContentCopy {
            builder: self,
            at,
            resources,
            kept: Vec::with_capacity(content.operations.len()),
            open: Vec::new(),
            hiding: 0,
            saved: 0,
            font_lost: at.no_font,
            font_saves: Vec::new(),
            in_text: false,
            state_fonts: Fresh::new(STATE_FONT_PREFIX),
            drawn: Fresh::new(DRAWN_PREFIX),
            drawn_names: BTreeMap::new(),
        };
        for operation in content.operations {
            copy.operation(operation)?;
        }
        copy.finish()
    }
}

/// The font a graphics state sets.
enum StateFont {
    /// It sets none.
    Unset,
    /// The `Tf` that selects it.
    Selected(Operation),
    /// It sets one that cannot be read or copied.
    Lost,
}

/// One content stream being rebuilt: the rebuild it copies into, where (the
/// scope its names resolve in), the resources they are copied into, the operations
/// kept, and what is open as it goes, so the content written is balanced and
/// ends in the state it began in.
struct ContentCopy<'c, 'a, 's> {
    builder: &'c mut Builder<'a>,
    at: At<'c, 's>,
    resources: &'c mut Dictionary,
    kept: Vec<Operation>,
    /// One entry per open marked-content sequence.
    open: Vec<Open>,
    /// How many of the open sequences hide, so whether content is hidden is
    /// known at once.
    hiding: usize,
    /// How many `q` saves are open.
    saved: usize,
    /// Whether there is no font to show text in (none is selected yet, or
    /// the one selected could not be copied), so the text shown is cut and
    /// reported; saved and restored with `q`/`Q`, as the font is graphics
    /// state.
    font_lost: bool,
    font_saves: Vec<bool>,
    /// Whether a text object is open.
    in_text: bool,
    /// The names fonts graphics states select are given.
    state_fonts: Fresh,
    /// The names a second copy of a form drawn under one name is given.
    drawn: Fresh,
    /// The name each XObject name and copy drawn is written under, so a
    /// name drawn again with the same copy is not named anew.
    drawn_names: BTreeMap<(Vec<u8>, ObjectId), Vec<u8>>,
}

impl<'a: 's, 's> ContentCopy<'_, 'a, 's> {
    /// Rebuild `operation`: keep it, rewrite it, or cut it.
    ///
    /// # Errors
    ///
    /// As [`Builder::copy`].
    fn operation(&mut self, operation: Operation) -> Result<()> {
        let operator = operation.operator.as_str();
        // What a hidden sequence draws is cut, but the graphics state it
        // sets lasts past it (8.11.3.2), so that is kept: a painting
        // operator ends its path unpainted, keeping any clip it sets, and a
        // text operator keeps its line move and spacing.
        if self.hiding > 0 {
            match operator {
                "BDC" | "BMC" => self.push(Marked::Within),
                "EMC" => {
                    self.pop();
                }
                "f" | "F" | "f*" | "S" | "s" | "B" | "B*" | "b" | "b*" => {
                    self.kept.push(Operation::new("n", Vec::new()));
                }
                "Tj" | "TJ" | "'" | "\"" if operands_fit(operator, &operation.operands) => {
                    self.text_state(&operation);
                }
                "Tj" | "TJ" | "'" | "\"" => self.builder.residue += 1,
                "sh" | "Do" | "BI" | "MP" | "DP" => {}
                _ => return self.visible(operation),
            }
            return Ok(());
        }
        self.visible(operation)
    }

    /// Rebuild `operation`, outside any hidden sequence or setting state
    /// within one.
    ///
    /// # Errors
    ///
    /// As [`Builder::copy`].
    fn visible(&mut self, operation: Operation) -> Result<()> {
        let operator = operation.operator.as_str();
        // A marked-content property list is rewritten, and an inline image is
        // cut, so only the operators kept as written are checked.
        if !matches!(operator, "BDC" | "DP" | "BI") && !operands_fit(operator, &operation.operands)
        {
            self.builder.residue += 1;
            return Ok(());
        }
        match operator {
            "BDC" | "DP" => self.marked(operation),
            "BMC" => {
                self.open_written();
                self.kept.push(operation);
            }
            "EMC" => {
                if self.pop().is_some() {
                    self.kept.push(operation);
                } else {
                    self.builder.residue += 1;
                }
            }
            "MP" => self.kept.push(operation),
            "BI" => self.builder.unreadable(None),
            "Tf" => self.font_lost = !self.resource(operation, Resource::Font)?,
            "gs" => {
                let font = self.state_font(&operation)?;
                self.resource(operation, Resource::ExtGState)?;
                match font {
                    StateFont::Unset => {}
                    StateFont::Selected(tf) => {
                        self.font_lost = false;
                        self.kept.push(tf);
                    }
                    StateFont::Lost => self.font_lost = true,
                }
            }
            "sh" => {
                self.resource(operation, Resource::Shading)?;
            }
            "cs" | "CS" => match operation.operands.first() {
                Some(Object::Name(name)) if FAMILIES.contains(&name.as_slice()) => {
                    self.kept.push(operation);
                }
                _ => {
                    self.resource(operation, Resource::ColorSpace)?;
                }
            },
            "scn" | "SCN" => match operation.operands.last() {
                Some(Object::Name(_)) => {
                    self.resource(operation, Resource::Pattern)?;
                }
                _ => self.kept.push(operation),
            },
            "Do" => self.draw(operation)?,
            "BX" | "EX" => {}
            "q" => {
                self.saved += 1;
                self.font_saves.push(self.font_lost);
                self.kept.push(operation);
            }
            "Q" if self.saved == 0 => self.builder.residue += 1,
            "Q" => {
                self.saved -= 1;
                self.font_lost = self.font_saves.pop().unwrap_or(false);
                self.kept.push(operation);
            }
            "Tj" | "TJ" | "'" | "\"" if self.font_lost => self.unreadable_text(&operation),
            "BT" if self.in_text => self.builder.residue += 1,
            "BT" => {
                self.in_text = true;
                self.kept.push(operation);
            }
            "ET" if !self.in_text => self.builder.residue += 1,
            "ET" => {
                self.close_text();
                self.kept.push(operation);
            }
            _ if OPERATORS.contains(&operator) => self.kept.push(operation),
            _ => self.builder.residue += 1,
        }
        Ok(())
    }

    /// Open a marked-content sequence written as `BMC`.
    fn open_written(&mut self) {
        self.push(Marked::Written);
    }

    /// Open a marked-content sequence of kind `marked`.
    fn push(&mut self, marked: Marked) {
        self.hiding += usize::from(marked == Marked::Hiding);
        self.open.push(Open {
            marked,
            in_text: self.in_text,
        });
    }

    /// Close the innermost marked-content sequence open, returning its kind.
    fn pop(&mut self) -> Option<Marked> {
        let marked = self.open.pop()?.marked;
        self.hiding -= usize::from(marked == Marked::Hiding);
        Some(marked)
    }

    /// End the open text object: each sequence written inside it closes
    /// first, with an `EMC`, so the output stays balanced. A hidden sequence
    /// begun inside it stays open, and hiding, until the source's own `EMC`:
    /// a misplaced `ET` does not show what the layer hides.
    fn close_text(&mut self) {
        let mut kept = Vec::with_capacity(self.open.len());
        let mut closed = 0;
        for open in self.open.drain(..).rev() {
            if open.in_text && open.marked == Marked::Written {
                closed += 1;
            } else {
                kept.push(Open {
                    in_text: false,
                    ..open
                });
            }
        }
        kept.reverse();
        self.open = kept;
        for _ in 0..closed {
            self.kept.push(Operation::new("EMC", Vec::new()));
        }
        self.in_text = false;
    }

    /// A `BDC` or `DP` `operation`: its property list dropped (`BDC` becomes
    /// `BMC`, `DP` becomes `MP`), and a sequence in a layer hidden by default
    /// cut whole; alternate text the list carried is recorded.
    fn marked(&mut self, operation: Operation) {
        let builder = &mut *self.builder;
        let is_sequence = operation.operator == "BDC";
        let [Object::Name(tag), properties] = operation.operands.as_slice() else {
            builder.residue += 1;
            if is_sequence {
                self.open_written();
                self.kept
                    .push(Operation::new("BMC", vec![Object::Name(b"Span".to_vec())]));
            }
            return;
        };
        let properties = match properties {
            Object::Name(name) => self.at.scope.lookup(builder.source, b"Properties", name),
            other => Some(other),
        };
        if is_sequence && tag == b"OC" && properties.is_some_and(|p| builder.layers.is_hidden(p)) {
            builder.record(HiddenContent::Layer);
            self.push(Marked::Hiding);
            return;
        }
        if properties
            .and_then(|p| builder.source.resolve_dict(p))
            .is_some_and(|p| ALTERNATE_TEXT_KEYS.iter().any(|k| p.get(k).is_ok()))
        {
            builder.record(HiddenContent::AlternativeText);
        }
        let tag = Object::Name(tag.clone());
        if is_sequence {
            self.open_written();
            self.kept.push(Operation::new("BMC", vec![tag]));
        } else {
            self.kept.push(Operation::new("MP", vec![tag]));
        }
    }

    /// Text shown in a font that could not be copied, cut and reported as
    /// unreadable; a line move or spacing it carries is kept, so the text
    /// after it stays where it was.
    fn unreadable_text(&mut self, operation: &Operation) {
        self.builder.record(HiddenContent::Unreadable);
        self.text_state(operation);
    }

    /// The line move and spacing the text-showing `operation` sets, without
    /// the text it shows.
    fn text_state(&mut self, operation: &Operation) {
        if operation.operator == "\"" {
            let [word, character, _] = operation.operands.as_slice() else {
                return;
            };
            self.kept.push(Operation::new("Tw", vec![word.clone()]));
            self.kept
                .push(Operation::new("Tc", vec![character.clone()]));
        }
        if matches!(operation.operator.as_str(), "'" | "\"") {
            self.kept.push(Operation::new("T*", Vec::new()));
        }
    }

    /// Keep `operation`, which names a `resource`, copying it into the
    /// resources; drop it when the name resolves to nothing kept. Whether it
    /// is kept.
    ///
    /// # Errors
    ///
    /// As [`Builder::copy`].
    fn resource(&mut self, operation: Operation, resource: Resource) -> Result<bool> {
        let category = resource.category();
        let Some(Object::Name(name)) = resource.name(&operation) else {
            self.builder.residue += 1;
            return Ok(false);
        };
        if !has(self.resources, category, name) {
            let Some(value) = self.at.scope.lookup(self.builder.source, category, name) else {
                self.builder.residue += 1;
                return Ok(false);
            };
            let Some(copy) = self
                .builder
                .copy(resource.kind(), value, self.at.deeper())?
            else {
                return Ok(false);
            };
            insert(self.resources, category, name, copy);
        }
        self.kept.push(operation);
        Ok(true)
    }

    /// Keep the `Do` `operation` when its XObject is kept: an image a codec
    /// reads, or a form, neither in a hidden layer.
    ///
    /// # Errors
    ///
    /// As [`Builder::copy`].
    fn draw(&mut self, operation: Operation) -> Result<()> {
        let builder = &mut *self.builder;
        let source = builder.source;
        let Some(Object::Name(name)) = operation.operands.first() else {
            builder.residue += 1;
            return Ok(());
        };
        let Some(value) = self.at.scope.lookup(source, b"XObject", name) else {
            builder.residue += 1;
            return Ok(());
        };
        let Some(Object::Stream(xobject)) = source.resolve(value) else {
            builder.residue += 1;
            return Ok(());
        };
        let id = value.as_reference().ok();
        if builder.layers.hides(&xobject.dict) {
            builder.record(HiddenContent::Layer);
            builder.cut.extend(id);
            return Ok(());
        }
        let kind = match xobject.dict.name(source, b"Subtype") {
            Some(b"Form") => Kind::Form,
            Some(b"Image") => Kind::Image,
            Some(b"PS") => {
                builder.record(HiddenContent::Script);
                builder.cut.extend(id);
                return Ok(());
            }
            _ => {
                builder.unreadable(id);
                return Ok(());
            }
        };
        // A form shows text before its own font in the caller's, so one
        // drawn where that font is lost is a copy of its own, under a name
        // of its own when the name already stands for the other copy.
        let at = self.at.deeper().font(self.font_lost);
        let Some(copy) = builder.copy(kind, value, at)? else {
            return Ok(());
        };
        let Ok(copied) = copy.as_reference() else {
            builder.residue += 1;
            return Ok(());
        };
        let key = (name.clone(), copied);
        let name = match self.drawn_names.get(&key) {
            Some(given) => given.clone(),
            None => {
                let existing = self
                    .resources
                    .get(b"XObject")
                    .and_then(Object::as_dict)
                    .ok()
                    .and_then(|entries| entries.get(name).ok());
                let given = match existing {
                    None => name.clone(),
                    Some(existing) if *existing == copy => name.clone(),
                    Some(_) => {
                        let resources = &*self.resources;
                        let scope = self.at.scope;
                        self.drawn.name(|name| {
                            has(resources, b"XObject", name)
                                || scope.lookup(source, b"XObject", name).is_some()
                        })
                    }
                };
                self.drawn_names.insert(key, given.clone());
                given
            }
        };
        insert(self.resources, b"XObject", &name, copy);
        self.kept
            .push(Operation::new("Do", vec![Object::Name(name)]));
        Ok(())
    }

    /// The font the graphics state `gs` names sets (`/Font [font size]`):
    /// selected by a `Tf`, its font copied into the resources under a name
    /// of its own, or lost when it cannot be read or copied. A graphics
    /// state's font is the same as a `Tf`, so writing it as one gives the
    /// text walk a single way fonts are selected.
    ///
    /// # Errors
    ///
    /// As [`Builder::copy`].
    fn state_font(&mut self, gs: &Operation) -> Result<StateFont> {
        let source = self.builder.source;
        let Some(Object::Name(name)) = gs.operands.first() else {
            return Ok(StateFont::Unset);
        };
        let Some(setting) = self
            .at
            .scope
            .lookup(source, b"ExtGState", name)
            .and_then(|state| source.resolve_dict(state))
            .and_then(|state| state.get(b"Font").ok())
        else {
            return Ok(StateFont::Unset);
        };
        let Some([font, size]) = source.resolve_array(setting).map(Vec::as_slice) else {
            self.builder.residue += 1;
            return Ok(StateFont::Lost);
        };
        let Some(size @ (Object::Integer(_) | Object::Real(_))) = source.resolve(size) else {
            self.builder.residue += 1;
            return Ok(StateFont::Lost);
        };
        let Some(font) = self.builder.copy(Kind::Font, font, self.at.deeper())? else {
            return Ok(StateFont::Lost);
        };
        // The font keeps the name it already has here, else takes the first
        // name nothing in scope uses.
        let resources = &*self.resources;
        let scope = self.at.scope;
        let taken = |name: &[u8]| {
            has(resources, b"Font", name) || scope.lookup(source, b"Font", name).is_some()
        };
        let name = resources
            .get(b"Font")
            .and_then(Object::as_dict)
            .ok()
            .and_then(|fonts| fonts.iter().find(|(_, value)| **value == font))
            .map(|(name, _)| name.clone())
            .unwrap_or_else(|| self.state_fonts.name(taken));
        insert(self.resources, b"Font", &name, font);
        Ok(StateFont::Selected(Operation::new(
            "Tf",
            vec![Object::Name(name), size.clone()],
        )))
    }

    /// The content written: what is open closed, inside out, and a default
    /// colour space the content's device spaces are remapped by copied.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if
    /// the content cannot be encoded, or as [`Builder::copy`].
    fn finish(mut self) -> Result<Vec<u8>> {
        // Only the sequences written are closed: one hidden to the end, and
        // any inside it, wrote no opener. Those begun in the open text object
        // close inside it.
        if self.in_text {
            self.close_text();
            self.kept.push(Operation::new("ET", Vec::new()));
        }
        while let Some(marked) = self.pop() {
            if marked == Marked::Written {
                self.kept.push(Operation::new("EMC", Vec::new()));
            }
        }
        for _ in 0..self.saved {
            self.kept.push(Operation::new("Q", Vec::new()));
        }
        // A default colour space remaps the device space of that name for
        // everything this content draws in it.
        for name in DEFAULT_SPACES {
            if !has(self.resources, b"ColorSpace", name)
                && let Some(space) = self
                    .at
                    .scope
                    .lookup(self.builder.source, b"ColorSpace", name)
                && let Some(space) = self
                    .builder
                    .copy(Kind::ColorSpace, space, self.at.deeper())?
            {
                insert(self.resources, b"ColorSpace", name, space);
            }
        }
        Content {
            operations: self.kept,
        }
        .encode()
        .map_err(|e| Error::malformed(format!("encode content: {e}")))
    }
}

/// Whether `operands` are what `operator` takes (8.2, table 50): operands of
/// another shape are not drawing, and a string among them is text no
/// detection reads.
fn operands_fit(operator: &str, operands: &[Object]) -> bool {
    let number = |o: &Object| matches!(o, Object::Integer(_) | Object::Real(_));
    let numbers = |count: usize| operands.len() == count && operands.iter().all(number);
    match operator {
        "q" | "Q" | "h" | "S" | "s" | "f" | "F" | "f*" | "B" | "B*" | "b" | "b*" | "n" | "W"
        | "W*" | "BT" | "ET" | "T*" | "EMC" | "BX" | "EX" => operands.is_empty(),
        "w" | "J" | "j" | "M" | "i" | "Tc" | "Tw" | "Tz" | "TL" | "Tr" | "Ts" | "G" | "g" => {
            numbers(1)
        }
        "m" | "l" | "Td" | "TD" | "d0" => numbers(2),
        "RG" | "rg" => numbers(3),
        "re" | "v" | "y" | "K" | "k" => numbers(4),
        "cm" | "c" | "Tm" | "d1" => numbers(6),
        "SC" | "sc" => !operands.is_empty() && operands.iter().all(number),
        "SCN" | "scn" => match operands.split_last() {
            Some((Object::Name(_), rest)) => rest.iter().all(number),
            Some(_) => operands.iter().all(number),
            None => false,
        },
        "ri" | "gs" | "sh" | "Do" | "CS" | "cs" | "BMC" | "MP" => {
            matches!(operands, [Object::Name(_)])
        }
        "Tf" => matches!(operands, [Object::Name(_), size] if number(size)),
        "d" => matches!(
            operands,
            [Object::Array(dashes), phase] if number(phase) && dashes.iter().all(number)
        ),
        "Tj" | "'" => matches!(operands, [Object::String(..)]),
        "\"" => matches!(operands, [w, c, Object::String(..)] if number(w) && number(c)),
        "TJ" => matches!(
            operands,
            [Object::Array(items)]
                if items.iter().all(|item| number(item) || matches!(item, Object::String(..)))
        ),
        _ => false,
    }
}

/// The names `prefix1`, `prefix2`, ..., handed out in turn: each index is
/// tried once, so naming many resources takes one pass, not one per name.
#[derive(Debug)]
pub(super) struct Fresh {
    prefix: &'static str,
    next: usize,
}

impl Fresh {
    /// Names starting from `prefix1`.
    pub(super) const fn new(prefix: &'static str) -> Self {
        Self { prefix, next: 1 }
    }

    /// The next name not `taken`.
    pub(super) fn name(&mut self, taken: impl Fn(&[u8]) -> bool) -> Vec<u8> {
        loop {
            let name = format!("{}{}", self.prefix, self.next).into_bytes();
            self.next += 1;
            if !taken(&name) {
                return name;
            }
        }
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
