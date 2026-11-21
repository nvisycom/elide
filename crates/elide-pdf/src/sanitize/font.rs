//! Fonts as the rebuild copies them: a Type3 font's glyph procedures
//! rebuilt as content, and a simple font's glyphs and widths pinned, so
//! every viewer selects and places them as the reader does.

use elide_core::Result;
use elide_core::redaction::HiddenContent;
use lopdf::{Dictionary, Object, dictionary};

use super::copy::At;
use super::keys::{FONT, Kind};
use super::{Builder, VALUE_COST};
use crate::pdf::reader::font::{Program, Standard, SymbolText, has_own_widths};
use crate::pdf::reader::{DictionaryExt, DocumentExt, StreamExt};
use crate::pdf::writer::compressed;

impl<'a> Builder<'a> {
    /// A font, a Type3 font's glyph procedures rebuilt as content with names
    /// resolved in its own resources, else in `at`'s scope. A simple font
    /// is given what the reader takes from its face or program: a standard
    /// face's glyphs and widths ([`pin`](Self::pin)), or what the program it
    /// embeds gives ([`pin_program`](Self::pin_program)), so every viewer
    /// selects and places its glyphs as the reader does.
    pub(super) fn font<'s>(&mut self, dict: &'s Dictionary, at: At<'_, 's>) -> Result<Object>
    where
        'a: 's,
    {
        let source = self.source;
        let subtype = dict.name(self.source, b"Subtype");
        let standard = Standard::of(source, dict);
        let handled: &[&[u8]] = match subtype {
            Some(b"Type3") => &[b"CharProcs", b"Resources", b"ToUnicode"],
            Some(b"Type0") => &[b"Encoding", b"ToUnicode"],
            // A pinned font's descriptor is its own (see `pin`).
            _ if standard.is_some() => &[b"FontDescriptor", b"ToUnicode"],
            _ => &[b"ToUnicode"],
        };
        let mut out = self.keys(dict, FONT, handled, at)?;
        // The text a viewer copies is written as the reader reads the font's.
        if let Ok(map) = dict.get(b"ToUnicode") {
            let kind = Kind::ToUnicode(SymbolText::of(source, dict));
            if let Some(map) = self.copy(kind, map, at.deeper())? {
                out.set("ToUnicode", map);
            }
        }
        if subtype == Some(b"Type0") {
            if let Ok(encoding) = dict.get(b"Encoding")
                && let Some(encoding) = self.copy(Kind::CMap, encoding, at.deeper())?
            {
                out.set("Encoding", encoding);
            }
            return Ok(Object::Dictionary(out));
        }
        if subtype != Some(b"Type3") {
            match standard {
                Some(standard) => {
                    self.pin(standard, dict, &mut out, at)?;
                    if !has_own_widths(source, dict) {
                        self.widths(&standard.widths(source, dict), &mut out)?;
                    }
                }
                None => self.pin_program(dict, &mut out)?,
            }
            return Ok(Object::Dictionary(out));
        }
        let inner = at.scope.inner(source, dict);
        let scoped = self.scoped;
        // The glyph procedures share one resource dictionary, so the names
        // they are rebuilt with are given across all of them.
        let mut resources = Dictionary::new();
        let mut procs = Dictionary::new();
        let glyphs = dict
            .get(b"CharProcs")
            .ok()
            .and_then(|p| source.resolve_dict(p));
        for (name, glyph) in glyphs.into_iter().flat_map(Dictionary::iter) {
            let Some(Object::Stream(glyph)) = source.resolve(glyph) else {
                self.residue += 1;
                continue;
            };
            if self.external(&glyph.dict) {
                self.record(HiddenContent::Unreadable);
                continue;
            }
            let content = self.content(
                &glyph.decoded()?,
                at.within(&inner).font(true),
                &mut resources,
            )?;
            self.charge(content.len())?;
            procs.set(
                name.clone(),
                self.out.add_object(compressed(Dictionary::new(), content)?),
            );
        }
        self.own_scope(dict, scoped);
        out.set("CharProcs", procs);
        out.set("Resources", resources);
        Ok(Object::Dictionary(out))
    }

    /// Pin the glyphs of `font`, which names the standard face `standard` and
    /// embeds no program, copied as `out`: an explicit `/Encoding` naming
    /// every code's glyph (`.notdef` for none), and a descriptor of its own
    /// whose flags make every reader take that encoding as it is (a TrueType
    /// font without one is given one). Every viewer then selects the glyphs
    /// the reader does, where without them each supplies its own. The descriptor is the font's own because its flags depend on
    /// the font: another font sharing the source's keeps a copy of its own.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::ResourceLimit`] if what it writes is past the copy budget.
    fn pin<'s>(
        &mut self,
        standard: Standard,
        font: &'s Dictionary,
        out: &mut Dictionary,
        at: At<'_, 's>,
    ) -> Result<()>
    where
        'a: 's,
    {
        let source = self.source;
        let names = standard.names(source, font);
        self.differences(names.iter().copied(), out)?;

        let descriptor = match font.get(b"FontDescriptor").ok().map(|d| source.resolve(d)) {
            Some(Some(descriptor @ Object::Dictionary(_))) => {
                match self.copy(Kind::FontDescriptor, descriptor, at.deeper())? {
                    Some(Object::Dictionary(mut copy)) => {
                        let flags = copy.get(b"Flags").and_then(Object::as_i64).unwrap_or(0);
                        copy.set("Flags", standard.flags(flags));
                        Some(copy)
                    }
                    _ => None,
                }
            }
            // A TrueType font with no descriptor reads as symbolic, which
            // ignores an `/Encoding` dictionary (9.6.5.4); give it one whose
            // flags make the encoding count.
            _ if font.is_subtype(source, b"TrueType") => {
                let name = font.name(source, b"BaseFont").unwrap_or(b"Font");
                let descriptor = standard.descriptor(name, standard.flags(0));
                self.charge(descriptor.len() * VALUE_COST)?;
                Some(descriptor)
            }
            _ => None,
        };
        if let Some(descriptor) = descriptor {
            out.set("FontDescriptor", self.out.add_object(descriptor));
        }
        Ok(())
    }

    /// Pin what the program the simple `font` embeds gives, copied as `out`:
    /// its widths, when the font gives none of its own, and, where a Type1
    /// font's encoding leaves codes to the program's built-in one, every
    /// code's glyph name as an explicit `/Encoding` (`.notdef` for none). The
    /// reader then reads the font as it is written, and never the program.
    ///
    /// The program is decoded within the copy budget, and charged to it, for
    /// each font that needs it: fonts sharing one program each read it.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::ResourceLimit`] if the program, or what is written, is
    /// past the copy budget.
    fn pin_program(&mut self, font: &Dictionary, out: &mut Dictionary) -> Result<()> {
        let source = self.source;
        if !Program::needed(source, font) {
            return Ok(());
        }
        let Some(program) = Program::embedded(source, font, self.copy_budget)? else {
            return Ok(());
        };
        self.charge(program.len())?;
        if !has_own_widths(source, font)
            && let Some(widths) = program.widths(source, font)
        {
            self.widths(&widths, out)?;
        }
        if let Some(names) = program.names(source, font) {
            self.differences(names.iter().map(Option::as_deref), out)?;
        }
        Ok(())
    }

    /// Pin `names`, one glyph name for each code (`.notdef` for none), as
    /// the explicit `/Encoding` of the font copied as `out`.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::ResourceLimit`] if it is past the copy budget.
    fn differences<'n>(
        &mut self,
        names: impl Iterator<Item = Option<&'n str>> + Clone,
        out: &mut Dictionary,
    ) -> Result<()> {
        let named: usize = names.clone().map(|name| name.map_or(7, str::len)).sum();
        self.charge((256 + 1) * VALUE_COST + named)?;
        let differences = [Object::Integer(0)]
            .into_iter()
            .chain(names.map(|name| Object::Name(name.unwrap_or(".notdef").as_bytes().to_vec())))
            .collect::<Vec<_>>();
        out.set(
            "Encoding",
            dictionary! { "Type" => "Encoding", "Differences" => differences },
        );
        Ok(())
    }

    /// Pin `widths`, one for each code, as the `/Widths` of the font copied as
    /// `out`, from `/FirstChar` 0 to `/LastChar` 255.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::ResourceLimit`] if they are past the copy budget.
    fn widths(&mut self, widths: &[f64; 256], out: &mut Dictionary) -> Result<()> {
        self.charge((widths.len() + 2) * VALUE_COST)?;
        out.set("FirstChar", 0);
        out.set("LastChar", 255);
        out.set("Widths", widths.map(number).to_vec());
        Ok(())
    }
}

/// `value` as a PDF number: an integer when it is whole.
fn number(value: f64) -> Object {
    if value.fract() == 0.0 && value.abs() < f64::from(i32::MAX) {
        Object::Integer(value as i64)
    } else {
        Object::Real(value as f32)
    }
}
