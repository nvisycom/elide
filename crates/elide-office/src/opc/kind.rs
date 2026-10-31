//! [`PartKind`]: what an OOXML package part is, read from its content type and
//! the relationship that reaches it, never guessed from its path.
//!
//! One table serves DOCX, PPTX, and XLSX: the three share OPC's packaging, its
//! metadata and embedding parts, and DrawingML, and differ only in the content
//! types of their own stories. A content type the table does not name is
//! [`Unknown`](PartKind::Unknown), and the sanitizer refuses a document holding
//! one rather than carry structure it cannot vouch for.

use elide_core::redaction::HiddenContent;

use crate::opc::block::EmbeddingKind;
use crate::opc::part::PartPath;

/// What a package part is, decided by its content type and the relationship
/// that reaches it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PartKind {
    /// A format's main part: the document body, the presentation, the workbook.
    Main,
    /// A part whose text is the document's visible content: a header, footer,
    /// footnote, glossary, slide, layout, master, notes page, worksheet, or the
    /// shared-string table.
    Story,
    /// A chart, its user shapes, or a chartEx chart.
    Chart,
    /// A SmartArt diagram's data or its rendered drawing.
    Diagram,
    /// A spreadsheet drawing, or a legacy VML drawing.
    Drawing,
    /// Comments and their threading, extension, and id parts.
    Comments,
    /// The people who wrote comments or revisions.
    CommentAuthors,
    /// A shared workbook's revision log.
    Revisions,
    /// An OPC relationships part.
    Relationships,
    /// A document-property part: core, extended (app), or custom properties.
    Properties,
    /// A custom XML data part or its properties.
    CustomXml,
    /// A binary embedding: an image, media clip, embedded object, or imported
    /// chunk.
    Embedding(EmbeddingKind),
    /// An embedded font.
    Font,
    /// A part that is hidden content in its entirety and always stripped,
    /// named by what it is: the thumbnail, a macro project, an ActiveX control,
    /// an add-in, a link to an external workbook, a data connection, printer
    /// settings, a signature, or application-private data.
    Hidden(HiddenContent),
    /// Structure that carries no user content: styles, themes, settings,
    /// numbering, fonts tables, properties of the view.
    Structure,
    /// A part this crate recognizes but cannot redact (a pivot cache, a slicer,
    /// a binary workbook), so a document holding one is refused.
    Unsupported,
    /// A part whose content type this crate does not recognize.
    Unknown,
}

/// The relationship type that names the package thumbnail.
const THUMBNAIL: &str =
    "http://schemas.openxmlformats.org/package/2006/relationships/metadata/thumbnail";

/// The relationship type of a WordprocessingML imported chunk (`w:altChunk`).
const IMPORTED_CHUNK: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/aFChunk";

/// The prefix of every WordprocessingML content type.
const WML: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.";
/// The prefix of every PresentationML content type.
const PML: &str = "application/vnd.openxmlformats-officedocument.presentationml.";
/// The prefix of every SpreadsheetML content type.
const SML: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.";

impl PartKind {
    /// Classify `part` from its `content_type` and the relationship types that
    /// reach it.
    pub(crate) fn of(part: &PartPath, content_type: Option<&str>, incoming: &[&str]) -> Self {
        // The thumbnail and imported chunks are typed by the relationship that
        // reaches them: their content types are a plain image or document type.
        if incoming.contains(&THUMBNAIL) {
            return Self::Hidden(HiddenContent::Thumbnail);
        }
        if incoming.contains(&IMPORTED_CHUNK) {
            return Self::Embedding(EmbeddingKind::Object);
        }
        // Custom XML items carry the generic XML content type; their place in the
        // package is what marks them.
        if part.in_dir("customXml") {
            return if part.is_relationships() {
                Self::Relationships
            } else {
                Self::CustomXml
            };
        }
        let Some(content_type) = content_type else {
            return Self::Unknown;
        };
        let content_type = content_type.to_ascii_lowercase();
        if let Some(kind) = Self::of_media(&content_type) {
            return kind;
        }
        if let Some(name) = content_type.strip_prefix(WML) {
            return Self::of_wordprocessing(name);
        }
        if let Some(name) = content_type.strip_prefix(PML) {
            return Self::of_presentation(name);
        }
        if let Some(name) = content_type.strip_prefix(SML) {
            return Self::of_spreadsheet(name);
        }
        Self::of_shared(&content_type)
    }

    /// The kind of an image, media, font, or embedded-object content type.
    fn of_media(content_type: &str) -> Option<Self> {
        let (family, _) = content_type.split_once('/')?;
        Some(match family {
            "image" => Self::Embedding(EmbeddingKind::Image),
            "audio" => Self::Embedding(EmbeddingKind::Audio),
            "video" => Self::Embedding(EmbeddingKind::Video),
            "font" => Self::Font,
            _ => match content_type {
                "application/vnd.openxmlformats-officedocument.obfuscatedfont"
                | "application/x-fontdata"
                | "application/x-font-ttf" => Self::Font,
                "application/vnd.openxmlformats-officedocument.oleobject"
                | "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
                | "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
                | "application/vnd.openxmlformats-officedocument.presentationml.presentation"
                | "application/vnd.ms-excel"
                | "application/msword"
                | "application/vnd.ms-powerpoint"
                | "application/pdf" => Self::Embedding(EmbeddingKind::Object),
                _ => return None,
            },
        })
    }

    /// The kind of a WordprocessingML content type, its `WML` prefix stripped.
    fn of_wordprocessing(name: &str) -> Self {
        match name {
            "document.main+xml" | "template.main+xml" => Self::Main,
            "header+xml"
            | "footer+xml"
            | "footnotes+xml"
            | "endnotes+xml"
            | "document.glossary+xml" => Self::Story,
            "comments+xml" => Self::Comments,
            "styles+xml" | "settings+xml" | "websettings+xml" | "fonttable+xml"
            | "numbering+xml" => Self::Structure,
            "printersettings" => Self::Hidden(HiddenContent::PrinterSettings),
            _ => Self::Unknown,
        }
    }

    /// The kind of a PresentationML content type, its `PML` prefix stripped.
    fn of_presentation(name: &str) -> Self {
        match name {
            "presentation.main+xml" | "slideshow.main+xml" | "template.main+xml" => Self::Main,
            "slide+xml" | "slidelayout+xml" | "slidemaster+xml" | "notesslide+xml"
            | "notesmaster+xml" | "handoutmaster+xml" => Self::Story,
            "comments+xml" => Self::Comments,
            "commentauthors+xml" => Self::CommentAuthors,
            "presprops+xml" | "viewprops+xml" | "tablestyles+xml" => Self::Structure,
            "tags+xml" => Self::Hidden(HiddenContent::ApplicationData),
            "printersettings" => Self::Hidden(HiddenContent::PrinterSettings),
            _ => Self::Unknown,
        }
    }

    /// The kind of a SpreadsheetML content type, its `SML` prefix stripped.
    fn of_spreadsheet(name: &str) -> Self {
        match name {
            "sheet.main+xml" | "template.main+xml" => Self::Main,
            "worksheet+xml" | "chartsheet+xml" | "dialogsheet+xml" | "sharedstrings+xml" => {
                Self::Story
            }
            "comments+xml" => Self::Comments,
            "revisionheaders+xml" | "revisionlog+xml" | "usernames+xml" => Self::Revisions,
            "styles+xml"
            | "calcchain+xml"
            | "table+xml"
            | "sheetmetadata+xml"
            | "volatiledependencies+xml" => Self::Structure,
            "externallink+xml" => Self::Hidden(HiddenContent::Link),
            "connections+xml" | "querytable+xml" => Self::Hidden(HiddenContent::DataConnection),
            "pivottable+xml" | "pivotcachedefinition+xml" | "pivotcacherecords+xml" => {
                Self::Unsupported
            }
            "printersettings" => Self::Hidden(HiddenContent::PrinterSettings),
            _ => Self::Unknown,
        }
    }

    /// The kind of a content type shared across the formats: OPC packaging,
    /// properties, DrawingML, and the Microsoft extensions.
    fn of_shared(content_type: &str) -> Self {
        match content_type {
            "application/vnd.openxmlformats-package.relationships+xml" => Self::Relationships,
            "application/vnd.openxmlformats-package.core-properties+xml"
            | "application/vnd.openxmlformats-officedocument.extended-properties+xml"
            | "application/vnd.openxmlformats-officedocument.custom-properties+xml" => {
                Self::Properties
            }
            "application/vnd.openxmlformats-officedocument.customxmlproperties+xml" => {
                Self::CustomXml
            }
            "application/vnd.openxmlformats-officedocument.theme+xml"
            | "application/vnd.openxmlformats-officedocument.themeoverride+xml"
            | "application/vnd.ms-office.chartstyle+xml"
            | "application/vnd.ms-office.chartcolorstyle+xml"
            | "application/vnd.openxmlformats-officedocument.drawingml.diagramlayout+xml"
            | "application/vnd.openxmlformats-officedocument.drawingml.diagramstyle+xml"
            | "application/vnd.openxmlformats-officedocument.drawingml.diagramcolors+xml"
            | "application/vnd.ms-word.styleswitheffects+xml" => Self::Structure,
            "application/vnd.openxmlformats-officedocument.drawingml.chart+xml"
            | "application/vnd.openxmlformats-officedocument.drawingml.chartshapes+xml"
            | "application/vnd.ms-office.chartex+xml" => Self::Chart,
            "application/vnd.openxmlformats-officedocument.drawingml.diagramdata+xml"
            | "application/vnd.ms-office.drawingml.diagramdrawing+xml" => Self::Diagram,
            "application/vnd.openxmlformats-officedocument.drawing+xml"
            | "application/vnd.openxmlformats-officedocument.vmldrawing" => Self::Drawing,
            "application/vnd.ms-word.commentsextended+xml"
            | "application/vnd.ms-word.commentsids+xml"
            | "application/vnd.ms-word.commentsextensible+xml"
            | "application/vnd.ms-excel.threadedcomments+xml"
            | "application/vnd.ms-powerpoint.comments+xml" => Self::Comments,
            "application/vnd.ms-word.people+xml"
            | "application/vnd.ms-excel.person+xml"
            | "application/vnd.ms-powerpoint.authors+xml" => Self::CommentAuthors,
            "application/vnd.ms-office.vbaproject"
            | "application/vnd.ms-office.vbaprojectsignature"
            | "application/vnd.ms-office.vbaprojectsignatureagile"
            | "application/vnd.ms-office.vbaprojectsignaturev3"
            | "application/vnd.ms-word.vbadata+xml"
            | "application/vnd.ms-word.keymapcustomizations+xml" => {
                Self::Hidden(HiddenContent::Macro)
            }
            "application/vnd.ms-office.activex+xml" | "application/vnd.ms-office.activex" => {
                Self::Hidden(HiddenContent::Control)
            }
            "application/vnd.ms-office.webextension+xml"
            | "application/vnd.ms-office.webextensiontaskpanes+xml" => {
                Self::Hidden(HiddenContent::AddIn)
            }
            "application/vnd.openxmlformats-package.digital-signature-origin"
            | "application/vnd.openxmlformats-package.digital-signature-xmlsignature+xml"
            | "application/vnd.openxmlformats-package.digital-signature-certificate" => {
                Self::Hidden(HiddenContent::Signature)
            }
            "application/vnd.ms-powerpoint.revisioninfo+xml" => {
                Self::Hidden(HiddenContent::EditingSessionId)
            }
            "application/vnd.ms-office.intelligence2+xml" => {
                Self::Hidden(HiddenContent::ApplicationData)
            }
            "application/vnd.ms-excel.slicer+xml"
            | "application/vnd.ms-excel.slicercache+xml"
            | "application/vnd.ms-excel.timeline+xml"
            | "application/vnd.ms-excel.timelinecache+xml"
            | "application/vnd.ms-excel.sheet.binary.macroenabled.main" => Self::Unsupported,
            "application/vnd.ms-word.document.macroenabled.main+xml"
            | "application/vnd.ms-word.template.macroenabledtemplate.main+xml"
            | "application/vnd.ms-powerpoint.presentation.macroenabled.main+xml"
            | "application/vnd.ms-powerpoint.slideshow.macroenabled.main+xml"
            | "application/vnd.ms-powerpoint.template.macroenabled.main+xml"
            | "application/vnd.ms-excel.sheet.macroenabled.main+xml"
            | "application/vnd.ms-excel.template.macroenabled.main+xml" => Self::Main,
            _ => Self::Unknown,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(path: &str, content_type: &str) -> PartKind {
        PartKind::of(&PartPath::from(path), Some(content_type), &[])
    }

    #[test]
    fn classifies_by_content_type_not_path() {
        let main = format!("{WML}document.main+xml");
        assert_eq!(kind("word/document.xml", &main), PartKind::Main);
        assert_eq!(kind("word/anything-at-all.xml", &main), PartKind::Main);
        assert_eq!(
            kind("word/comments.xml", &format!("{WML}comments+xml")),
            PartKind::Comments
        );
        assert_eq!(
            kind("word/people.xml", "application/vnd.ms-word.people+xml"),
            PartKind::CommentAuthors
        );
        assert_eq!(
            kind("word/media/image1.emf", "image/x-emf"),
            PartKind::Embedding(EmbeddingKind::Image)
        );
        assert_eq!(
            kind(
                "word/vbaProject.bin",
                "application/vnd.ms-office.vbaProject"
            ),
            PartKind::Hidden(HiddenContent::Macro)
        );
    }

    #[test]
    fn an_unrecognized_or_missing_content_type_is_unknown() {
        assert_eq!(
            kind("word/new.xml", "application/vnd.ms-word.future+xml"),
            PartKind::Unknown
        );
        assert_eq!(
            PartKind::of(&PartPath::from("word/x.bin"), None, &[]),
            PartKind::Unknown
        );
    }

    #[test]
    fn the_thumbnail_and_custom_xml_are_known_by_their_place() {
        let thumbnail = PartKind::of(
            &PartPath::from("docProps/thumbnail.jpeg"),
            Some("image/jpeg"),
            &[THUMBNAIL],
        );
        assert_eq!(thumbnail, PartKind::Hidden(HiddenContent::Thumbnail));
        assert_eq!(
            kind("customXml/item1.xml", "application/xml"),
            PartKind::CustomXml
        );
    }
}
