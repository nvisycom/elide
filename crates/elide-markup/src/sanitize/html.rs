//! The HTML allowlist: the elements and attributes a browser renders, in
//! HTML and in the SVG and MathML it embeds, and what anything else is
//! recorded as when the rebuild leaves it out.
//!
//! Element and attribute names are compared lowercased, as a browser matches
//! them in an HTML document.

use std::collections::HashSet;

use elide_core::redaction::{HiddenContent, HiddenContentPolicy};

use super::url::{SCRIPT_SCHEMES, css_loads_remote, is_remote, loads_remote, normalized_url};
use crate::engine::meta::MetaTag;
use crate::engine::tag::HtmlAttribute;
use crate::references::{CharacterReferences, decode_references};

/// The vocabulary an element is read in: HTML, or the SVG or MathML a
/// document embeds with an `<svg>` or `<math>` element.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Space {
    Html,
    Svg,
    Math,
}

/// HTML elements that render: the document structure, the elements of a
/// page's body, and the presentational ones a browser still renders. A
/// `<meta>`, `<link>`, or `<style>` is kept only when it neither describes nor
/// loads (see [`element`]).
const HTML_ELEMENTS: &[&str] = &[
    "a",
    "abbr",
    "acronym",
    "address",
    "area",
    "article",
    "aside",
    "audio",
    "b",
    "bdi",
    "bdo",
    "big",
    "blockquote",
    "body",
    "br",
    "button",
    "canvas",
    "caption",
    "center",
    "cite",
    "code",
    "col",
    "colgroup",
    "data",
    "datalist",
    "dd",
    "del",
    "details",
    "dfn",
    "dialog",
    "dir",
    "div",
    "dl",
    "dt",
    "em",
    "fieldset",
    "figcaption",
    "figure",
    "font",
    "footer",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "head",
    "header",
    "hgroup",
    "hr",
    "html",
    "i",
    "img",
    "input",
    "ins",
    "kbd",
    "label",
    "legend",
    "li",
    "link",
    "main",
    "map",
    "mark",
    "menu",
    "meta",
    "meter",
    "nav",
    "nobr",
    "noscript",
    "ol",
    "optgroup",
    "option",
    "output",
    "p",
    "picture",
    "pre",
    "progress",
    "q",
    "rp",
    "rt",
    "ruby",
    "s",
    "samp",
    "search",
    "section",
    "select",
    "small",
    "source",
    "span",
    "strike",
    "strong",
    "style",
    "sub",
    "summary",
    "sup",
    "table",
    "tbody",
    "td",
    "textarea",
    "tfoot",
    "th",
    "thead",
    "time",
    "title",
    "tr",
    "track",
    "tt",
    "u",
    "ul",
    "var",
    "video",
    "wbr",
];

/// HTML elements that run or embed active content: a script, a plugin object,
/// and a frame that loads another page or carries one inline.
const ACTIVE_ELEMENTS: &[&str] = &[
    "applet",
    "embed",
    "fencedframe",
    "frame",
    "frameset",
    "iframe",
    "object",
    "portal",
];

/// HTML void elements: they have no end tag, so leaving one out leaves out its
/// tag alone.
pub(super) const VOID_ELEMENTS: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

/// The HTML elements that end SVG or MathML when met inside it, so a browser
/// reads them, and what follows, as HTML (WHATWG HTML §13.2.6.5).
const BREAKOUT_ELEMENTS: &[&str] = &[
    "b",
    "big",
    "blockquote",
    "body",
    "br",
    "center",
    "code",
    "dd",
    "div",
    "dl",
    "dt",
    "em",
    "embed",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "head",
    "hr",
    "i",
    "img",
    "li",
    "listing",
    "menu",
    "meta",
    "nobr",
    "ol",
    "p",
    "pre",
    "ruby",
    "s",
    "small",
    "span",
    "strike",
    "strong",
    "sub",
    "sup",
    "table",
    "tt",
    "u",
    "ul",
    "var",
];

/// SVG elements that draw, or define what is drawn, lowercased.
const SVG_ELEMENTS: &[&str] = &[
    "a",
    "circle",
    "clippath",
    "defs",
    "desc",
    "ellipse",
    "feblend",
    "fecolormatrix",
    "fecomponenttransfer",
    "fecomposite",
    "feconvolvematrix",
    "fediffuselighting",
    "fedisplacementmap",
    "fedistantlight",
    "fedropshadow",
    "feflood",
    "fefunca",
    "fefuncb",
    "fefuncg",
    "fefuncr",
    "fegaussianblur",
    "feimage",
    "femerge",
    "femergenode",
    "femorphology",
    "feoffset",
    "fepointlight",
    "fespecularlighting",
    "fespotlight",
    "fetile",
    "feturbulence",
    "filter",
    "g",
    "image",
    "line",
    "lineargradient",
    "marker",
    "mask",
    "path",
    "pattern",
    "polygon",
    "polyline",
    "radialgradient",
    "rect",
    "stop",
    "style",
    "svg",
    "switch",
    "symbol",
    "text",
    "textpath",
    "title",
    "tspan",
    "use",
    "view",
];

/// SVG elements that animate: each can set another element's attributes, a
/// link's target among them, as the document runs.
const SVG_ANIMATIONS: &[&str] = &["animate", "animatemotion", "animatetransform", "set"];

/// MathML elements that render.
const MATH_ELEMENTS: &[&str] = &[
    "math",
    "menclose",
    "merror",
    "mfenced",
    "mfrac",
    "mi",
    "mlabeledtr",
    "mmultiscripts",
    "mn",
    "mo",
    "mover",
    "mpadded",
    "mprescripts",
    "mroot",
    "mrow",
    "ms",
    "mspace",
    "msqrt",
    "mstyle",
    "msub",
    "msubsup",
    "msup",
    "mtable",
    "mtd",
    "mtext",
    "mtr",
    "munder",
    "munderover",
    "none",
    "semantics",
];

/// MathML elements that never show: a phantom only takes up its room, and an
/// annotation describes the expression for software.
const MATH_UNDISPLAYED: &[&str] = &["annotation", "annotation-xml", "mphantom"];

/// Attributes every HTML element may carry. `hidden` is kept only as
/// `until-found`, which a reader can still find and show; `aria-*` attributes
/// are read below.
const HTML_GLOBAL: &[&str] = &[
    "accesskey",
    "autocapitalize",
    "autofocus",
    "class",
    "contenteditable",
    "dir",
    "draggable",
    "enterkeyhint",
    "hidden",
    "id",
    "inert",
    "inputmode",
    "lang",
    "popover",
    "role",
    "spellcheck",
    "style",
    "tabindex",
    "title",
    "translate",
    "xml:lang",
    "xmlns",
];

/// Attributes every SVG element may carry: its geometry, presentation, and
/// references to the definitions it draws with.
const SVG_GLOBAL: &[&str] = &[
    "alignment-baseline",
    "baseline-shift",
    "class",
    "clip-path",
    "clip-rule",
    "clippathunits",
    "color",
    "color-interpolation",
    "color-interpolation-filters",
    "cx",
    "cy",
    "d",
    "direction",
    "dominant-baseline",
    "dx",
    "dy",
    "fill",
    "fill-opacity",
    "fill-rule",
    "filter",
    "filterunits",
    "flood-color",
    "flood-opacity",
    "font-family",
    "font-size",
    "font-size-adjust",
    "font-stretch",
    "font-style",
    "font-variant",
    "font-weight",
    "fr",
    "fx",
    "fy",
    "gradienttransform",
    "gradientunits",
    "height",
    "href",
    "id",
    "image-rendering",
    "isolation",
    "lang",
    "lengthadjust",
    "letter-spacing",
    "marker-end",
    "marker-mid",
    "marker-start",
    "markerheight",
    "markerunits",
    "markerwidth",
    "mask",
    "maskcontentunits",
    "maskunits",
    "mix-blend-mode",
    "opacity",
    "orient",
    "overflow",
    "paint-order",
    "pathlength",
    "patterncontentunits",
    "patterntransform",
    "patternunits",
    "points",
    "preserveaspectratio",
    "primitiveunits",
    "r",
    "refx",
    "refy",
    "role",
    "rotate",
    "rx",
    "ry",
    "shape-rendering",
    "spreadmethod",
    "startoffset",
    "stop-color",
    "stop-opacity",
    "stroke",
    "stroke-dasharray",
    "stroke-dashoffset",
    "stroke-linecap",
    "stroke-linejoin",
    "stroke-miterlimit",
    "stroke-opacity",
    "stroke-width",
    "style",
    "tabindex",
    "text-anchor",
    "text-decoration",
    "text-rendering",
    "textlength",
    "transform",
    "transform-origin",
    "vector-effect",
    "version",
    "viewbox",
    "visibility",
    "width",
    "word-spacing",
    "writing-mode",
    "x",
    "x1",
    "x2",
    "xlink:href",
    "xml:lang",
    "xml:space",
    "xmlns",
    "xmlns:xlink",
    "y",
    "y1",
    "y2",
];

/// The attributes of SVG's filter primitives.
const SVG_FILTER: &[&str] = &[
    "amplitude",
    "azimuth",
    "basefrequency",
    "bias",
    "diffuseconstant",
    "divisor",
    "edgemode",
    "elevation",
    "exponent",
    "in",
    "in2",
    "intercept",
    "k1",
    "k2",
    "k3",
    "k4",
    "kernelmatrix",
    "kernelunitlength",
    "limitingconeangle",
    "mode",
    "numoctaves",
    "offset",
    "operator",
    "order",
    "pointsatx",
    "pointsaty",
    "pointsatz",
    "preservealpha",
    "radius",
    "result",
    "scale",
    "seed",
    "slope",
    "specularconstant",
    "specularexponent",
    "stddeviation",
    "stitchtiles",
    "surfacescale",
    "tablevalues",
    "targetx",
    "targety",
    "type",
    "values",
    "xchannelselector",
    "ychannelselector",
    "z",
];

/// Attributes every MathML element may carry.
const MATH_GLOBAL: &[&str] = &[
    "accent",
    "accentunder",
    "align",
    "class",
    "close",
    "columnalign",
    "columnlines",
    "columnspacing",
    "columnspan",
    "depth",
    "dir",
    "display",
    "displaystyle",
    "fence",
    "frame",
    "height",
    "id",
    "largeop",
    "linethickness",
    "lspace",
    "mathbackground",
    "mathcolor",
    "mathsize",
    "mathvariant",
    "maxsize",
    "minsize",
    "movablelimits",
    "notation",
    "open",
    "rowalign",
    "rowlines",
    "rowspacing",
    "rowspan",
    "rspace",
    "scriptlevel",
    "separator",
    "separators",
    "stretchy",
    "style",
    "symmetric",
    "voffset",
    "width",
    "xmlns",
];

/// The attributes an HTML element carries besides the global ones.
fn html_attributes(element: &str) -> &'static [&'static str] {
    match element {
        "a" => &[
            "download",
            "href",
            "hreflang",
            "referrerpolicy",
            "rel",
            "target",
            "type",
        ],
        "area" => &[
            "alt",
            "coords",
            "download",
            "href",
            "referrerpolicy",
            "rel",
            "shape",
            "target",
        ],
        "audio" => &[
            "autoplay",
            "controls",
            "crossorigin",
            "loop",
            "muted",
            "preload",
            "src",
        ],
        "blockquote" | "q" => &["cite"],
        "body" => &["alink", "background", "bgcolor", "link", "text", "vlink"],
        "br" => &["clear"],
        "button" => &[
            "disabled",
            "form",
            "formaction",
            "formenctype",
            "formmethod",
            "formnovalidate",
            "formtarget",
            "name",
            "popovertarget",
            "popovertargetaction",
            "type",
            "value",
        ],
        "canvas" => &["height", "width"],
        "caption" | "legend" | "div" | "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => &["align"],
        "col" | "colgroup" => &["align", "span", "valign", "width"],
        "data" => &["value"],
        "del" | "ins" => &["cite", "datetime"],
        "details" => &["name", "open"],
        "dialog" => &["open"],
        "fieldset" => &["disabled", "form", "name"],
        "font" => &["color", "face", "size"],
        "form" => &[
            "accept-charset",
            "action",
            "autocomplete",
            "enctype",
            "method",
            "name",
            "novalidate",
            "rel",
            "target",
        ],
        "hr" => &["align", "color", "noshade", "size", "width"],
        "html" => &["version"],
        "img" => &[
            "align",
            "alt",
            "border",
            "crossorigin",
            "decoding",
            "fetchpriority",
            "height",
            "hspace",
            "ismap",
            "loading",
            "referrerpolicy",
            "sizes",
            "src",
            "srcset",
            "usemap",
            "vspace",
            "width",
        ],
        "input" => &[
            "accept",
            "alt",
            "autocomplete",
            "checked",
            "dirname",
            "disabled",
            "form",
            "formaction",
            "formenctype",
            "formmethod",
            "formnovalidate",
            "formtarget",
            "height",
            "list",
            "max",
            "maxlength",
            "min",
            "minlength",
            "multiple",
            "name",
            "pattern",
            "placeholder",
            "popovertarget",
            "popovertargetaction",
            "readonly",
            "required",
            "size",
            "src",
            "step",
            "type",
            "value",
            "width",
        ],
        "label" => &["for"],
        "li" => &["type", "value"],
        "link" => &[
            "as",
            "crossorigin",
            "href",
            "hreflang",
            "media",
            "referrerpolicy",
            "rel",
            "sizes",
            "type",
        ],
        "map" => &["name"],
        "meta" => &[
            "charset",
            "content",
            "http-equiv",
            "itemprop",
            "name",
            "property",
        ],
        "meter" => &["high", "low", "max", "min", "optimum", "value"],
        "ol" => &["reversed", "start", "type"],
        "optgroup" => &["disabled", "label"],
        "option" => &["disabled", "label", "selected", "value"],
        "output" => &["for", "form", "name"],
        "pre" => &["width"],
        "progress" => &["max", "value"],
        "select" => &[
            "autocomplete",
            "disabled",
            "form",
            "multiple",
            "name",
            "required",
            "size",
        ],
        "source" => &["height", "media", "sizes", "src", "srcset", "type", "width"],
        "style" => &["media", "type"],
        "table" => &[
            "align",
            "bgcolor",
            "border",
            "cellpadding",
            "cellspacing",
            "frame",
            "rules",
            "summary",
            "width",
        ],
        "tbody" | "tfoot" | "thead" | "tr" => &["align", "bgcolor", "valign"],
        "td" | "th" => &[
            "abbr", "align", "bgcolor", "colspan", "headers", "height", "nowrap", "rowspan",
            "scope", "valign", "width",
        ],
        "textarea" => &[
            "autocomplete",
            "cols",
            "dirname",
            "disabled",
            "form",
            "maxlength",
            "minlength",
            "name",
            "placeholder",
            "readonly",
            "required",
            "rows",
            "wrap",
        ],
        "time" => &["datetime"],
        "track" => &["default", "kind", "label", "src", "srclang"],
        "ul" => &["type"],
        "video" => &[
            "autoplay",
            "controls",
            "crossorigin",
            "height",
            "loop",
            "muted",
            "playsinline",
            "poster",
            "preload",
            "src",
            "width",
        ],
        _ => &[],
    }
}

/// The attributes whose value is a URL the page loads when it is opened. A
/// hyperlink's `href` is not among them: a link a reader follows is visible
/// content.
const LOADING_ATTRIBUTES: &[&str] = &[
    "action",
    "background",
    "data",
    "formaction",
    "icon",
    "longdesc",
    "manifest",
    "poster",
    "src",
    "srcset",
];

/// The `http-equiv` directives that act when the page opens: a redirect, and a
/// cookie set on the reader's browser.
const ACTING_DIRECTIVES: [&str; 2] = ["refresh", "set-cookie"];

/// What the rebuild does with an element.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Element {
    /// Copy it, with the attributes [`attribute`] keeps.
    Keep,
    /// Copy it, entering the vocabulary it opens (`<svg>`, `<math>`).
    Enter(Space),
    /// Leave it out with everything inside it, as the hidden content it is.
    Drop(HiddenContent),
}

/// What the rebuild does with the element `name` (lowercased) read in `space`,
/// with `attributes` as written, under `policy`.
pub(super) fn element(
    space: Space,
    name: &str,
    attributes: &[HtmlAttribute<'_>],
    policy: HiddenContentPolicy,
) -> Element {
    let value = |key: &str| {
        attributes
            .iter()
            .find(|attr| attr.name.eq_ignore_ascii_case(key))
            .map(|attr| keyword(attr.value.unwrap_or_default()))
    };
    if value("hidden").is_some_and(|hidden| hidden != "until-found") {
        return Element::Drop(HiddenContent::Undisplayed);
    }
    match space {
        Space::Html => html_element(name, attributes, &value, policy),
        Space::Svg => match name {
            "script" => Element::Drop(HiddenContent::Script),
            _ if SVG_ANIMATIONS.contains(&name) => Element::Drop(HiddenContent::Action),
            "metadata" => Element::Drop(HiddenContent::Properties),
            _ if undisplayed_svg(&value) => Element::Drop(HiddenContent::Undisplayed),
            _ if SVG_ELEMENTS.contains(&name) => Element::Keep,
            _ => Element::Drop(HiddenContent::ApplicationData),
        },
        Space::Math => match name {
            _ if MATH_UNDISPLAYED.contains(&name) => Element::Drop(HiddenContent::Undisplayed),
            _ if MATH_ELEMENTS.contains(&name) => Element::Keep,
            _ => Element::Drop(HiddenContent::ApplicationData),
        },
    }
}

/// [`element`] in HTML, `value` reading an attribute as a browser compares it.
fn html_element(
    name: &str,
    attributes: &[HtmlAttribute<'_>],
    value: &dyn Fn(&str) -> Option<String>,
    policy: HiddenContentPolicy,
) -> Element {
    match name {
        "script" => Element::Drop(HiddenContent::Script),
        _ if ACTIVE_ELEMENTS.contains(&name) => Element::Drop(HiddenContent::Control),
        "template" => Element::Drop(HiddenContent::Undisplayed),
        "base" => Element::Drop(HiddenContent::Link),
        "link"
            if attributes.iter().any(|attr| {
                attr.name.eq_ignore_ascii_case("href") && attr.value.is_some_and(is_remote)
            }) =>
        {
            Element::Drop(HiddenContent::Link)
        }
        // Writing a repeated attribute once could turn a field into a tag that
        // renders the page, whose content is then never flagged, so such a
        // tag goes whole.
        "meta" if repeats(attributes) => Element::Drop(HiddenContent::Properties),
        "meta" => match meta_removal(attributes, policy) {
            Some(content) => Element::Drop(content),
            None => Element::Keep,
        },
        // A dialog shows only once opened by script, which never runs.
        "dialog" if value("open").is_none() => Element::Drop(HiddenContent::Undisplayed),
        "input" if value("type").as_deref() == Some("hidden") => {
            Element::Drop(HiddenContent::Undisplayed)
        }
        // The root of SVG is read in SVG, where `display` is a property.
        "svg" if undisplayed_svg(value) => Element::Drop(HiddenContent::Undisplayed),
        "svg" => Element::Enter(Space::Svg),
        "math" => Element::Enter(Space::Math),
        _ if HTML_ELEMENTS.contains(&name) => Element::Keep,
        _ => Element::Drop(HiddenContent::ApplicationData),
    }
}

/// Whether an SVG element whose attributes `value` reads is not displayed:
/// its `display` property, read as CSS, is `none`.
fn undisplayed_svg(value: &dyn Fn(&str) -> Option<String>) -> bool {
    value("display").is_some_and(|display| display.trim() == "none")
}

/// Whether an attribute of `attributes` is written twice.
fn repeats(attributes: &[HtmlAttribute<'_>]) -> bool {
    let mut seen = HashSet::with_capacity(attributes.len());
    !attributes
        .iter()
        .all(|attr| seen.insert(attr.name.to_ascii_lowercase()))
}

/// What a `<meta>` tag with `attributes` is left out as, or `None` when it
/// stays: a directive that acts on opening goes whatever the policy, one that
/// renders the page stays, and any other field is metadata.
fn meta_removal(
    attributes: &[HtmlAttribute<'_>],
    policy: HiddenContentPolicy,
) -> Option<HiddenContent> {
    let tag = MetaTag::new(
        attributes
            .iter()
            .map(|attr| (attr.name, attr.value.unwrap_or_default())),
    );
    if tag
        .directive()
        .is_some_and(|directive| ACTING_DIRECTIVES.contains(&directive))
    {
        return Some(HiddenContent::Action);
    }
    (!tag.renders() && !policy.metadata.treats_as_content()).then_some(HiddenContent::Properties)
}

/// Whether the allowlist names the element `name` (lowercased) in `space`,
/// whatever its attributes.
pub(super) fn names(space: Space, name: &str) -> bool {
    match space {
        Space::Html => HTML_ELEMENTS.contains(&name) || name == "svg" || name == "math",
        Space::Svg => SVG_ELEMENTS.contains(&name),
        Space::Math => MATH_ELEMENTS.contains(&name),
    }
}

/// Whether the element `name` in `space` ends SVG or MathML, so a browser
/// reads it as HTML.
pub(super) fn breaks_out(space: Space, name: &str, attributes: &[HtmlAttribute<'_>]) -> bool {
    if space == Space::Html {
        return false;
    }
    BREAKOUT_ELEMENTS.contains(&name)
        || (name == "font"
            && attributes.iter().any(|attr| {
                ["color", "face", "size"]
                    .iter()
                    .any(|key| attr.name.eq_ignore_ascii_case(key))
            }))
}

/// What the attribute `attr` of the element `element` (lowercased) read in
/// `space` is left out as, or `None` when it is copied: an `on*` event
/// handler or a URL that runs script, a URL the page loads from a server, a
/// `ping` that reports a followed link, inline style or an SVG presentation
/// attribute that loads from a server, and anything else that no reader of
/// the page sees.
pub(super) fn attribute(
    space: Space,
    element: &str,
    attr: &HtmlAttribute<'_>,
) -> Option<HiddenContent> {
    let key = attr.name.to_ascii_lowercase();
    let local = key.rsplit(':').next().unwrap_or(&key);
    let value = attr.value.unwrap_or_default();
    let url = normalized_url(value);
    if key.starts_with("on") || SCRIPT_SCHEMES.iter().any(|scheme| url.starts_with(scheme)) {
        return Some(HiddenContent::Script);
    }
    let hyperlink = element == "a" || element == "area";
    if local == "ping"
        || (LOADING_ATTRIBUTES.contains(&local) && loads_remote(value, local))
        || (local == "href" && !hyperlink && is_remote(value))
        || (local == "style" && css_loads_remote(&url))
        // An SVG presentation attribute is CSS, whose `url()` loads too.
        || (space == Space::Svg && css_loads_remote(&url))
    {
        return Some(HiddenContent::Link);
    }
    let allowed = match space {
        Space::Html => {
            HTML_GLOBAL.contains(&key.as_str()) || html_attributes(element).contains(&key.as_str())
        }
        Space::Svg => {
            SVG_GLOBAL.contains(&key.as_str())
                || (element.starts_with("fe") && SVG_FILTER.contains(&key.as_str()))
        }
        Space::Math => MATH_GLOBAL.contains(&key.as_str()),
    };
    // Accessible names and descriptions are read to the reader by assistive
    // technology, and extracted like any other attribute value.
    (!allowed && !key.starts_with("aria-")).then_some(HiddenContent::ApplicationData)
}

/// An attribute value as a browser compares an enumerated keyword: character
/// references decoded and lowercased, but not trimmed, so `" hidden "` is no
/// keyword at all.
fn keyword(value: &str) -> String {
    let (decoded, _) = decode_references(value, 0, CharacterReferences::Html);
    decoded.to_ascii_lowercase()
}

/// The HTML elements whose body a browser reads as text with references
/// decoded (RCDATA), never as markup.
pub(super) const RCDATA_ELEMENTS: [&str; 2] = ["textarea", "title"];

/// The HTML elements whose body a browser reads as text up to their end tag,
/// never as markup: the RCDATA ones and the raw-text ones. A `<plaintext>`
/// has no end tag, so its text runs to the end of the document.
pub(super) const TEXT_ELEMENTS: [&str; 10] = [
    "iframe",
    "noembed",
    "noframes",
    "noscript",
    "plaintext",
    "script",
    "style",
    "textarea",
    "title",
    "xmp",
];

/// Whether the SVG or MathML element `name` (lowercased) read in `space` is
/// one whose content a browser reads as HTML: an SVG `foreignObject`, `desc`,
/// or `title`, and a MathML token element.
pub(super) fn integration_point(space: Space, name: &str) -> bool {
    match space {
        Space::Html => false,
        Space::Svg => matches!(name, "foreignobject" | "desc" | "title"),
        Space::Math => matches!(name, "mi" | "mo" | "mn" | "ms" | "mtext"),
    }
}

/// Whether the element `name` (lowercased), started in a MathML token
/// element, stays MathML where any other element is read as HTML.
pub(super) fn stays_math(name: &str) -> bool {
    matches!(name, "mglyph" | "malignmark")
}

/// Whether the start tag `start` (lowercased) closes an open HTML element
/// `open` that holds nothing else open: the end tags HTML lets a document
/// leave out.
pub(super) fn closes(open: &str, start: &str) -> bool {
    match open {
        "p" => P_CLOSERS.contains(&start),
        "li" => start == "li",
        "dt" | "dd" => matches!(start, "dt" | "dd"),
        "option" => matches!(start, "option" | "optgroup"),
        "optgroup" => start == "optgroup",
        "rb" | "rt" | "rp" | "rtc" => matches!(start, "rb" | "rt" | "rp" | "rtc"),
        "tr" => start == "tr",
        "td" | "th" => matches!(start, "td" | "th" | "tr"),
        "thead" | "tbody" | "tfoot" => matches!(start, "thead" | "tbody" | "tfoot"),
        _ => false,
    }
}

/// The start tags that close an open `<p>`.
const P_CLOSERS: &[&str] = &[
    "address",
    "article",
    "aside",
    "blockquote",
    "center",
    "dd",
    "details",
    "dialog",
    "dir",
    "div",
    "dl",
    "dt",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hgroup",
    "hr",
    "li",
    "listing",
    "main",
    "menu",
    "nav",
    "ol",
    "p",
    "plaintext",
    "pre",
    "search",
    "section",
    "summary",
    "table",
    "ul",
    "xmp",
];
