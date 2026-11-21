//! Synthetic font programs: an sfnt and a bare CFF program holding only what
//! a reader takes advance widths and glyph names from.

use lopdf::{Dictionary, Document, Stream, dictionary};

/// An sfnt (TrueType) program: named glyphs with advances, glyph 0 being
/// `.notdef`, and `cmap` subtables.
pub(crate) struct Sfnt {
    units_per_em: u16,
    glyphs: Vec<(&'static str, u16)>,
    cmaps: Vec<Cmap>,
}

/// A format 6 `cmap` subtable: the codes from `first` on mapped to `glyphs`.
struct Cmap {
    platform: u16,
    encoding: u16,
    first: u16,
    glyphs: Vec<u16>,
}

impl Sfnt {
    /// A program of `units_per_em` to the em with `glyphs`, each a name and
    /// an advance; the first is `.notdef`.
    pub(crate) fn new(units_per_em: u16, glyphs: &[(&'static str, u16)]) -> Self {
        Self {
            units_per_em,
            glyphs: glyphs.to_vec(),
            cmaps: Vec::new(),
        }
    }

    /// A `cmap` subtable for `platform` and `encoding` mapping the codes from
    /// `first` on to `glyphs`.
    pub(crate) fn cmap(mut self, platform: u16, encoding: u16, first: u16, glyphs: &[u16]) -> Self {
        self.cmaps.push(Cmap {
            platform,
            encoding,
            first,
            glyphs: glyphs.to_vec(),
        });
        self
    }

    /// The program's bytes.
    pub(crate) fn build(&self) -> Vec<u8> {
        let count = self.glyphs.len() as u16;
        let mut head = vec![0; 54];
        head[0..4].copy_from_slice(&0x0001_0000_u32.to_be_bytes());
        head[12..16].copy_from_slice(&0x5F0F_3CF5_u32.to_be_bytes());
        head[18..20].copy_from_slice(&self.units_per_em.to_be_bytes());
        let mut hhea = vec![0; 36];
        hhea[0..4].copy_from_slice(&0x0001_0000_u32.to_be_bytes());
        hhea[34..36].copy_from_slice(&count.to_be_bytes());
        let mut maxp = 0x0000_5000_u32.to_be_bytes().to_vec();
        maxp.extend(count.to_be_bytes());
        let hmtx: Vec<u8> = self
            .glyphs
            .iter()
            .flat_map(|(_, advance)| [advance.to_be_bytes(), [0, 0]].concat())
            .collect();

        let mut cmap = Vec::new();
        cmap.extend(0_u16.to_be_bytes());
        cmap.extend((self.cmaps.len() as u16).to_be_bytes());
        let mut subtables = Vec::new();
        let mut offset = 4 + 8 * self.cmaps.len();
        for subtable in &self.cmaps {
            cmap.extend(subtable.platform.to_be_bytes());
            cmap.extend(subtable.encoding.to_be_bytes());
            cmap.extend((offset as u32).to_be_bytes());
            let count = subtable.glyphs.len() as u16;
            let mut table = Vec::new();
            for value in [6, 10 + 2 * count, 0, subtable.first, count] {
                table.extend(value.to_be_bytes());
            }
            for glyph in &subtable.glyphs {
                table.extend(glyph.to_be_bytes());
            }
            offset += table.len();
            subtables.extend(table);
        }
        cmap.extend(subtables);

        let mut post = 0x0002_0000_u32.to_be_bytes().to_vec();
        post.extend([0; 28]);
        post.extend(count.to_be_bytes());
        post.extend(0_u16.to_be_bytes());
        for index in 1..count {
            post.extend((257 + index).to_be_bytes());
        }
        for (name, _) in &self.glyphs[1..] {
            post.push(name.len() as u8);
            post.extend(name.as_bytes());
        }

        let tables: [(&[u8; 4], Vec<u8>); 6] = [
            (b"cmap", cmap),
            (b"head", head),
            (b"hhea", hhea),
            (b"hmtx", hmtx),
            (b"maxp", maxp),
            (b"post", post),
        ];
        let mut out = 0x0001_0000_u32.to_be_bytes().to_vec();
        out.extend((tables.len() as u16).to_be_bytes());
        out.extend([0; 6]);
        let mut offset = 12 + 16 * tables.len();
        for (tag, data) in &tables {
            out.extend(*tag);
            out.extend([0; 4]);
            out.extend((offset as u32).to_be_bytes());
            out.extend((data.len() as u32).to_be_bytes());
            offset += data.len().next_multiple_of(4);
        }
        for (_, data) in tables {
            out.extend(data);
            out.resize(out.len().next_multiple_of(4), 0);
        }
        out
    }
}

/// A bare CFF program: named glyphs with advances, glyph 0 being `.notdef`,
/// and a built-in encoding giving glyph `i` the `i`th code of `codes`.
pub(crate) struct Cff {
    glyphs: Vec<(&'static str, i16)>,
    codes: Vec<u8>,
}

impl Cff {
    /// A program with `glyphs`, each a name and an advance in thousandths of
    /// an em; the first is `.notdef`.
    pub(crate) fn new(glyphs: &[(&'static str, i16)]) -> Self {
        Self {
            glyphs: glyphs.to_vec(),
            codes: Vec::new(),
        }
    }

    /// The built-in encoding: glyph `i + 1` at `codes[i]`.
    pub(crate) fn encoding(mut self, codes: &[u8]) -> Self {
        self.codes = codes.to_vec();
        self
    }

    /// The program's bytes.
    pub(crate) fn build(&self) -> Vec<u8> {
        let header = [1, 0, 4, 2];
        let name = index(&[b"Test".to_vec()]);
        let strings = index(
            &self.glyphs[1..]
                .iter()
                .map(|(name, _)| name.as_bytes().to_vec())
                .collect::<Vec<_>>(),
        );
        let global_subrs = index(&[]);

        let mut charset = vec![0];
        for (custom, (name, _)) in (391..).zip(&self.glyphs[1..]) {
            charset.extend(standard_sid(name).unwrap_or(custom).to_be_bytes());
        }
        let mut encoding = vec![0, self.codes.len() as u8];
        encoding.extend(&self.codes);
        let char_strings = index(
            &self
                .glyphs
                .iter()
                .map(|(_, advance)| {
                    let [high, low] = advance.to_be_bytes();
                    vec![28, high, low, 14]
                })
                .collect::<Vec<_>>(),
        );
        let private = vec![139, 20, 139, 21];

        // Every offset is a 5-byte integer, so the Top DICT is as long
        // whatever the offsets are, and its length can be taken first.
        let top = |charset_at: usize| {
            let encoding_at = charset_at + charset.len();
            let char_strings_at = encoding_at + encoding.len();
            let private_at = char_strings_at + char_strings.len();
            let mut top = Vec::new();
            for (operands, operator) in [
                (vec![charset_at], 15),
                (vec![encoding_at], 16),
                (vec![char_strings_at], 17),
                (vec![private.len(), private_at], 18),
            ] {
                for operand in operands {
                    top.push(29);
                    top.extend((operand as i32).to_be_bytes());
                }
                top.push(operator);
            }
            index(&[top])
        };
        let charset_at =
            header.len() + name.len() + top(0).len() + strings.len() + global_subrs.len();
        let top = top(charset_at);

        [
            header.to_vec(),
            name,
            top,
            strings,
            global_subrs,
            charset,
            encoding,
            char_strings,
            private,
        ]
        .concat()
    }
}

/// A Type 1 program: named glyphs with advances, glyph 0 being `.notdef`,
/// and a built-in encoding giving each code its glyph's name.
pub(crate) struct Type1 {
    glyphs: Vec<(&'static str, i32)>,
    codes: Vec<(u8, &'static str)>,
}

impl Type1 {
    /// A program with `glyphs`, each a name and an advance in thousandths of
    /// an em; the first is `.notdef`.
    pub(crate) fn new(glyphs: &[(&'static str, i32)]) -> Self {
        Self {
            glyphs: glyphs.to_vec(),
            codes: Vec::new(),
        }
    }

    /// The built-in encoding: each code with the name of its glyph.
    pub(crate) fn encoding(mut self, codes: &[(u8, &'static str)]) -> Self {
        self.codes = codes.to_vec();
        self
    }

    /// The program's bytes: its font dictionary in the clear, its private
    /// dictionary eexec-encrypted as hex, and its charstrings unencrypted.
    pub(crate) fn build(&self) -> Vec<u8> {
        let mut base = String::from(
            "%!PS-AdobeFont-1.0: Test\n/FontName /Test def \
             /FontMatrix [0.001 0 0 0.001 0 0] readonly def\n\
             /Encoding 256 array 0 1 255 {1 index exch /.notdef put} for\n",
        );
        for (code, name) in &self.codes {
            base.push_str(&format!("dup {code} /{name} put\n"));
        }
        base.push_str("readonly def\ncurrentfile eexec\n");

        let mut private = format!(
            "/lenIV -1 def /CharStrings {} dict dup begin\n",
            self.glyphs.len()
        )
        .into_bytes();
        for (name, advance) in &self.glyphs {
            // `0 advance hsbw endchar`.
            let mut charstring = type1_number(0);
            charstring.extend(type1_number(*advance));
            charstring.extend([13, 14]);
            private.extend(format!("/{name} {} RD ", charstring.len()).as_bytes());
            private.extend(charstring);
            private.extend(b" ND\n");
        }
        private.extend(b"end");

        let mut out = base.into_bytes();
        let mut key: u32 = 55665;
        for plain in [0u8; 4].iter().chain(&private) {
            let cipher = (u32::from(*plain) ^ (key >> 8)) as u8;
            key = (u32::from(cipher).wrapping_add(key))
                .wrapping_mul(52845)
                .wrapping_add(22719)
                & 0xFFFF;
            out.extend(format!("{cipher:02x}").as_bytes());
        }
        out
    }
}

/// A Type 1 charstring number.
fn type1_number(value: i32) -> Vec<u8> {
    match value {
        -107..=107 => vec![(value + 139) as u8],
        108..=1131 => {
            let value = value - 108;
            vec![(value / 256 + 247) as u8, (value % 256) as u8]
        }
        -1131..=-108 => {
            let value = -value - 108;
            vec![(value / 256 + 251) as u8, (value % 256) as u8]
        }
        _ => [&[255], value.to_be_bytes().as_slice()].concat(),
    }
}

/// The standard string id of the glyph `name`, for the few names tests use;
/// a reader looks a standard name up by it.
fn standard_sid(name: &str) -> Option<u16> {
    match name {
        "space" => Some(1),
        "A" => Some(34),
        "B" => Some(35),
        "a" => Some(66),
        "b" => Some(67),
        _ => None,
    }
}

/// A CFF INDEX of `items`, with 2-byte offsets.
fn index(items: &[Vec<u8>]) -> Vec<u8> {
    let mut out = (items.len() as u16).to_be_bytes().to_vec();
    if items.is_empty() {
        return out;
    }
    out.push(2);
    let mut offset = 1_u16;
    out.extend(offset.to_be_bytes());
    for item in items {
        offset += item.len() as u16;
        out.extend(offset.to_be_bytes());
    }
    for item in items {
        out.extend(item);
    }
    out
}

/// The font file an embedded program is kept in.
#[derive(Clone, Copy)]
pub(crate) enum FontFile {
    /// `/FontFile2`: a TrueType program.
    TrueType,
    /// `/FontFile3` of subtype `OpenType`.
    OpenType,
    /// `/FontFile3` of subtype `Type1C`: a bare CFF program.
    Cff,
    /// `/FontFile`: a Type 1 program.
    Type1,
}

impl FontFile {
    /// The descriptor key the program is under, and its stream's subtype.
    fn key(self) -> (&'static str, Option<&'static str>) {
        match self {
            FontFile::TrueType => ("FontFile2", None),
            FontFile::OpenType => ("FontFile3", Some("OpenType")),
            FontFile::Cff => ("FontFile3", Some("Type1C")),
            FontFile::Type1 => ("FontFile", None),
        }
    }
}

/// A simple font of `subtype` whose descriptor, flagged `flags`, embeds
/// `program` in `doc` as `file`.
pub(crate) fn embedding(
    doc: &mut Document,
    subtype: &str,
    file: FontFile,
    program: Vec<u8>,
    flags: i64,
) -> Dictionary {
    let (key, kind) = file.key();
    let mut dict = Dictionary::new();
    if let Some(kind) = kind {
        dict.set("Subtype", kind);
    }
    let program = doc.add_object(Stream::new(dict, program));
    let mut descriptor = dictionary! {
        "Type" => "FontDescriptor",
        "FontName" => "ABCDEF+Embedded",
        "Flags" => flags,
    };
    descriptor.set(key, program);
    dictionary! {
        "Type" => "Font",
        "Subtype" => subtype,
        "BaseFont" => "ABCDEF+Embedded",
        "FontDescriptor" => descriptor,
    }
}
