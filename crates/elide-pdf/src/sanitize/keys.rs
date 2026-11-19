//! What each kind of object keeps: the keys the PDF specification defines for
//! it, each with the kind of value it holds. A key not listed is not copied.

/// The kind of a value being copied, which decides the keys it keeps and how
/// its children are copied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum Kind {
    /// A number, name, string, or boolean, or an array of them.
    Value,
    /// A font dictionary (any subtype; a Type3 font's glyph procedures are
    /// rebuilt as content).
    Font,
    /// A descendant CID font of a Type0 font.
    CidFont,
    /// A font descriptor.
    FontDescriptor,
    /// A font descriptor's `/Style`.
    FontStyle,
    /// A CID font's character collection.
    CidSystemInfo,
    /// A font's encoding dictionary (or the name of one).
    Encoding,
    /// An embedded font program.
    FontProgram,
    /// A CMap stream (a Type0 font's encoding, a `/UseCMap` base), or its
    /// name.
    CMap,
    /// A font's `/ToUnicode` CMap stream: the text a viewer copies.
    ToUnicode,
    /// A stream of opaque data: a palette, a CID set, a CID-to-glyph map,
    /// global JBIG2 segments.
    Data,
    /// An image XObject.
    Image,
    /// An image's `/Mask`: a stencil image, or an array of colour ranges.
    Mask,
    /// A form XObject, rebuilt as content.
    Form,
    /// A pattern; a tiling one is rebuilt as content.
    Pattern,
    /// A transparency group attribute dictionary.
    Group,
    /// A graphics state parameter dictionary.
    ExtGState,
    /// A soft mask dictionary (or `/None`).
    SoftMask,
    /// A colour space: a family name, or an array naming the family and its
    /// parameters.
    ColorSpace,
    /// A CIE-based colour space's parameters.
    CalParams,
    /// An ICC profile stream.
    IccProfile,
    /// A function (or a name such as `/Identity`).
    Function,
    /// A shading.
    Shading,
    /// A stream's filter parameters.
    DecodeParms,
    /// A file specification, its file given a neutral name.
    FileSpec,
    /// An embedded file stream.
    EmbeddedFile,
    /// An embedded file's parameters.
    FileParams,
    /// A DeviceN colour space's attributes.
    DeviceNAttributes,
    /// A DeviceN space's process colour space and components.
    Process,
    /// A DeviceN space's mixing hints.
    MixingHints,
    /// An annotation's border style.
    BorderStyle,
    /// An output intent: the colour target the document is made for.
    OutputIntent,
    /// A dictionary whose every value is of the kind given: a CIDFont's
    /// per-class font descriptors, DeviceN colorants, mixing solidities and
    /// dot gains.
    Map(&'static Kind),
}

/// The kinds a [`Kind::Map`] holds.
pub(super) const FONT_DESCRIPTORS: Kind = Kind::Map(&Kind::FontDescriptor);
pub(super) const COLOR_SPACES: Kind = Kind::Map(&Kind::ColorSpace);
pub(super) const VALUES: Kind = Kind::Map(&Kind::Value);
pub(super) const FUNCTIONS: Kind = Kind::Map(&Kind::Function);

use Kind::{
    CMap, CidFont, CidSystemInfo, ColorSpace, Data, DecodeParms, Encoding, FileParams, FileSpec,
    FontDescriptor, FontProgram, FontStyle, Form, Function, Image, Mask, SoftMask, ToUnicode,
    Value,
};

/// The keys every stream keeps beside its own: how its data is encoded. The
/// length is rewritten on save.
pub(super) const STREAM: &[(&[u8], Kind)] = &[
    (b"Filter", Value),
    (b"DecodeParms", DecodeParms),
    (b"DL", Value),
];

/// The keys a table-driven `kind` keeps, or `None` for a kind copied by hand
/// (fonts, forms, patterns, colour spaces, file specifications).
pub(super) fn fields(kind: Kind) -> Option<&'static [(&'static [u8], Kind)]> {
    Some(match kind {
        Kind::CidFont => &[
            (b"Type", Value),
            (b"Subtype", Value),
            (b"BaseFont", Value),
            (b"CIDSystemInfo", CidSystemInfo),
            (b"FontDescriptor", FontDescriptor),
            (b"DW", Value),
            (b"W", Value),
            (b"DW2", Value),
            (b"W2", Value),
            (b"CIDToGIDMap", Data),
        ],
        Kind::FontDescriptor => &[
            (b"Type", Value),
            (b"FontName", Value),
            (b"FontFamily", Value),
            (b"FontStretch", Value),
            (b"FontWeight", Value),
            (b"Flags", Value),
            (b"FontBBox", Value),
            (b"ItalicAngle", Value),
            (b"Ascent", Value),
            (b"Descent", Value),
            (b"Leading", Value),
            (b"CapHeight", Value),
            (b"XHeight", Value),
            (b"StemV", Value),
            (b"StemH", Value),
            (b"AvgWidth", Value),
            (b"MaxWidth", Value),
            (b"MissingWidth", Value),
            (b"CharSet", Value),
            (b"Lang", Value),
            (b"Style", FontStyle),
            (b"FontFile", FontProgram),
            (b"FontFile2", FontProgram),
            (b"FontFile3", FontProgram),
            (b"CIDSet", Data),
            (b"FD", FONT_DESCRIPTORS),
        ],
        Kind::FontStyle => &[(b"Panose", Value)],
        Kind::CidSystemInfo => &[
            (b"Registry", Value),
            (b"Ordering", Value),
            (b"Supplement", Value),
        ],
        Kind::Encoding => &[
            (b"Type", Value),
            (b"BaseEncoding", Value),
            (b"Differences", Value),
        ],
        Kind::FontProgram => &[
            (b"Subtype", Value),
            (b"Length1", Value),
            (b"Length2", Value),
            (b"Length3", Value),
        ],
        Kind::CMap => CMAP,
        Kind::ToUnicode => TO_UNICODE,
        Kind::Data => &[],
        Kind::Image => &[
            (b"Type", Value),
            (b"Subtype", Value),
            (b"Width", Value),
            (b"Height", Value),
            (b"ColorSpace", ColorSpace),
            (b"BitsPerComponent", Value),
            (b"Intent", Value),
            (b"ImageMask", Value),
            (b"Mask", Mask),
            (b"SMask", Image),
            (b"SMaskInData", Value),
            (b"Decode", Value),
            (b"Interpolate", Value),
            (b"Matte", Value),
            (b"AF", FileSpec),
        ],
        Kind::Mask => &[
            (b"Type", Value),
            (b"Subtype", Value),
            (b"Width", Value),
            (b"Height", Value),
            (b"BitsPerComponent", Value),
            (b"ImageMask", Value),
            (b"Decode", Value),
            (b"Interpolate", Value),
        ],
        Kind::Group => &[
            (b"Type", Value),
            (b"S", Value),
            (b"CS", ColorSpace),
            (b"I", Value),
            (b"K", Value),
        ],
        Kind::ExtGState => &[
            (b"Type", Value),
            (b"LW", Value),
            (b"LC", Value),
            (b"LJ", Value),
            (b"ML", Value),
            (b"D", Value),
            (b"RI", Value),
            (b"OP", Value),
            (b"op", Value),
            (b"OPM", Value),
            (b"FL", Value),
            (b"SM", Value),
            (b"SA", Value),
            (b"BM", Value),
            (b"CA", Value),
            (b"ca", Value),
            (b"AIS", Value),
            (b"TK", Value),
            (b"UseBlackPtComp", Value),
            (b"BG", Function),
            (b"BG2", Function),
            (b"UCR", Function),
            (b"UCR2", Function),
            (b"TR", Function),
            (b"TR2", Function),
            (b"SMask", SoftMask),
        ],
        Kind::SoftMask => &[
            (b"Type", Value),
            (b"S", Value),
            (b"BC", Value),
            (b"G", Form),
            (b"TR", Function),
        ],
        Kind::CalParams => &[
            (b"WhitePoint", Value),
            (b"BlackPoint", Value),
            (b"Gamma", Value),
            (b"Matrix", Value),
            (b"Range", Value),
        ],
        Kind::IccProfile => &[(b"N", Value), (b"Range", Value), (b"Alternate", ColorSpace)],
        Kind::Function => &[
            (b"FunctionType", Value),
            (b"Domain", Value),
            (b"Range", Value),
            (b"Size", Value),
            (b"BitsPerSample", Value),
            (b"Order", Value),
            (b"Encode", Value),
            (b"Decode", Value),
            (b"C0", Value),
            (b"C1", Value),
            (b"N", Value),
            (b"Bounds", Value),
            (b"Functions", Function),
        ],
        Kind::Shading => &[
            (b"ShadingType", Value),
            (b"ColorSpace", ColorSpace),
            (b"Background", Value),
            (b"BBox", Value),
            (b"AntiAlias", Value),
            (b"Coords", Value),
            (b"Domain", Value),
            (b"Function", Function),
            (b"Extend", Value),
            (b"Matrix", Value),
            (b"BitsPerCoordinate", Value),
            (b"BitsPerComponent", Value),
            (b"BitsPerFlag", Value),
            (b"Decode", Value),
            (b"VerticesPerRow", Value),
        ],
        Kind::DecodeParms => &[
            (b"Predictor", Value),
            (b"Colors", Value),
            (b"BitsPerComponent", Value),
            (b"Columns", Value),
            (b"EarlyChange", Value),
            (b"K", Value),
            (b"EndOfLine", Value),
            (b"EncodedByteAlign", Value),
            (b"Rows", Value),
            (b"EndOfBlock", Value),
            (b"BlackIs1", Value),
            (b"DamagedRowsBeforeError", Value),
            (b"ColorTransform", Value),
            (b"JBIG2Globals", Data),
        ],
        Kind::EmbeddedFile => &[
            (b"Type", Value),
            (b"Subtype", Value),
            (b"Params", FileParams),
        ],
        Kind::FileParams => &[(b"Size", Value)],
        Kind::DeviceNAttributes => &[
            (b"Subtype", Value),
            (b"Colorants", COLOR_SPACES),
            (b"Process", Kind::Process),
            (b"MixingHints", Kind::MixingHints),
        ],
        Kind::Process => &[(b"ColorSpace", ColorSpace), (b"Components", Value)],
        Kind::MixingHints => &[
            (b"Solidities", VALUES),
            (b"PrintingOrder", Value),
            (b"DotGain", FUNCTIONS),
        ],
        Kind::BorderStyle => &[
            (b"Type", Value),
            (b"W", Value),
            (b"S", Value),
            (b"D", Value),
        ],
        Kind::OutputIntent => &[
            (b"Type", Value),
            (b"S", Value),
            (b"OutputCondition", Value),
            (b"OutputConditionIdentifier", Value),
            (b"RegistryName", Value),
            (b"Info", Value),
            (b"DestOutputProfile", Kind::IccProfile),
            (b"MixingHints", Kind::MixingHints),
        ],
        Kind::Value
        | Kind::Font
        | Kind::Form
        | Kind::Pattern
        | Kind::ColorSpace
        | Kind::FileSpec
        | Kind::Map(_) => return None,
    })
}

/// The keys a font keeps; a Type3 font's `/CharProcs` and `/Resources` are
/// rebuilt as content beside these, and a Type0 font's `/Encoding` is a CMap.
pub(super) const FONT: &[(&[u8], Kind)] = &[
    (b"Type", Value),
    (b"Subtype", Value),
    (b"BaseFont", Value),
    (b"Name", Value),
    (b"FirstChar", Value),
    (b"LastChar", Value),
    (b"Widths", Value),
    (b"FontDescriptor", FontDescriptor),
    (b"Encoding", Encoding),
    (b"ToUnicode", ToUnicode),
    (b"DescendantFonts", CidFont),
    (b"FontBBox", Value),
    (b"FontMatrix", Value),
];

/// The keys a CMap keeps.
pub(super) const CMAP: &[(&[u8], Kind)] = &[
    (b"Type", Value),
    (b"CMapName", Value),
    (b"CIDSystemInfo", CidSystemInfo),
    (b"WMode", Value),
    (b"UseCMap", CMap),
];

/// The keys a `/ToUnicode` CMap keeps: a `/UseCMap` base is dropped, since
/// the map is written from the CMap's own entries, which detection reads
/// text with, but a viewer would follow it.
pub(super) const TO_UNICODE: &[(&[u8], Kind)] = &[
    (b"Type", Value),
    (b"CMapName", Value),
    (b"CIDSystemInfo", CidSystemInfo),
    (b"WMode", Value),
];

/// The keys a form XObject keeps beside its rebuilt content and resources.
pub(super) const FORM: &[(&[u8], Kind)] = &[
    (b"Type", Value),
    (b"Subtype", Value),
    (b"FormType", Value),
    (b"BBox", Value),
    (b"Matrix", Value),
    (b"Group", Kind::Group),
    (b"AF", FileSpec),
];

/// The keys a tiling pattern keeps beside its rebuilt content and resources.
pub(super) const TILING: &[(&[u8], Kind)] = &[
    (b"Type", Value),
    (b"PatternType", Value),
    (b"PaintType", Value),
    (b"TilingType", Value),
    (b"BBox", Value),
    (b"XStep", Value),
    (b"YStep", Value),
    (b"Matrix", Value),
];

/// The keys a shading pattern keeps.
pub(super) const SHADING_PATTERN: &[(&[u8], Kind)] = &[
    (b"Type", Value),
    (b"PatternType", Value),
    (b"Shading", Kind::Shading),
    (b"Matrix", Value),
    (b"ExtGState", Kind::ExtGState),
];
