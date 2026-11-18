//! Page geometry: where a page's boxes lie and how it is displayed, and the
//! glyph boxes on it, in default user space and as fractions of the page as
//! displayed.

use std::ops::Range;

use elide_core::primitive::UnitBoundingBox;
use lopdf::{Document, ObjectId};

use super::DocumentExt;

/// One glyph's box and the decoded character range it drew, so a detected
/// character span can be resolved back to the page rectangles it covers.
#[derive(Debug, Clone)]
pub(crate) struct GlyphRect {
    /// The page-local decoded character range this glyph produced.
    pub(crate) chars: Range<usize>,
    /// The glyph's box in default user space, bottom-left origin.
    pub(crate) rect: DeviceRect,
}

/// An axis-aligned box in device (page) space, as the glyph quad projects to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct DeviceRect {
    pub(crate) min_x: f64,
    pub(crate) min_y: f64,
    pub(crate) max_x: f64,
    pub(crate) max_y: f64,
}

impl DeviceRect {
    /// The box's four corners, as `(x, y)` points.
    #[cfg(feature = "render")]
    pub(crate) fn corners(&self) -> [(f64, f64); 4] {
        [
            (self.min_x, self.min_y),
            (self.max_x, self.min_y),
            (self.max_x, self.max_y),
            (self.min_x, self.max_y),
        ]
    }

    /// The axis-aligned bounds of four device-space corners (a glyph quad mapped
    /// through a possibly-rotated matrix is not axis-aligned; take its extent).
    pub(crate) fn of_corners(corners: [(f64, f64); 4]) -> DeviceRect {
        let xs = corners.map(|(x, _)| x);
        let ys = corners.map(|(_, y)| y);
        DeviceRect {
            min_x: xs.iter().copied().fold(f64::INFINITY, f64::min),
            min_y: ys.iter().copied().fold(f64::INFINITY, f64::min),
            max_x: xs.iter().copied().fold(f64::NEG_INFINITY, f64::max),
            max_y: ys.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        }
    }

    /// The union covering both boxes, for merging a line's glyphs into one rect.
    fn union(self, other: DeviceRect) -> DeviceRect {
        DeviceRect {
            min_x: self.min_x.min(other.min_x),
            min_y: self.min_y.min(other.min_y),
            max_x: self.max_x.max(other.max_x),
            max_y: self.max_y.max(other.max_y),
        }
    }
}

/// The visible page rectangle and its display rotation, for normalizing a
/// device-space [`DeviceRect`] into a page-normalized [`UnitBoundingBox`].
///
/// The walker's device coordinates are in unrotated PDF user space (bottom-left
/// origin). A viewer displays the page's `CropBox` region, rotated
/// by [`rotate`](Self::rotate) degrees, so normalization must (1) place the crop
/// origin at zero, (2) divide by the crop size, (3) apply the rotation, and
/// (4) flip Y to the top-left origin a renderer uses.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PageFrame {
    /// CropBox lower-left corner in user space (fallback: MediaBox).
    crop_x0: f64,
    crop_y0: f64,
    /// CropBox width and height in user space (always positive).
    crop_w: f64,
    crop_h: f64,
    /// Display rotation, normalized to one of 0, 90, 180, 270.
    rotate: u32,
}

impl PageFrame {
    /// The frame of a page with no readable box, as a viewer draws it: US
    /// Letter, unrotated.
    #[cfg(feature = "render")]
    pub(crate) const LETTER: Self = Self {
        crop_x0: 0.0,
        crop_y0: 0.0,
        crop_w: 612.0,
        crop_h: 792.0,
        rotate: 0,
    };
    /// The frame of a page with no readable box: the unit square, unrotated,
    /// so geometry on it stays unnormalized rather than failing.
    pub(crate) const UNIT: Self = Self {
        crop_x0: 0.0,
        crop_y0: 0.0,
        crop_w: 1.0,
        crop_h: 1.0,
        rotate: 0,
    };

    /// Normalize a device-space box into a page-normalized, top-left-origin
    /// [`UnitBoundingBox`] in the as-displayed (rotated) frame.
    fn normalize(&self, rect: DeviceRect) -> UnitBoundingBox {
        // Guard degenerate crop dimensions so normalization never divides by zero.
        let cw = self.crop_w.max(f64::MIN_POSITIVE);
        let ch = self.crop_h.max(f64::MIN_POSITIVE);

        // Step 1+2: crop-relative fractions, still bottom-left origin, unrotated.
        let fx0 = (rect.min_x - self.crop_x0) / cw;
        let fx1 = (rect.max_x - self.crop_x0) / cw;
        let fy0 = (rect.min_y - self.crop_y0) / ch;
        let fy1 = (rect.max_y - self.crop_y0) / ch;

        // Step 3+4: rotate the two corners into the displayed frame, then flip Y
        // to a top-left origin. Rotation maps the unrotated unit square to the
        // displayed one; we rotate both corners and re-derive the min/extent.
        let p0 = rotate_unit_point(fx0, fy0, self.rotate);
        let p1 = rotate_unit_point(fx1, fy1, self.rotate);
        let (min_x, max_x) = (p0.0.min(p1.0), p0.0.max(p1.0));
        // Bottom-left-origin y in the rotated frame → top-left by `1 - y`.
        let top0 = 1.0 - p0.1;
        let top1 = 1.0 - p1.1;
        let (min_y, max_y) = (top0.min(top1), top0.max(top1));

        UnitBoundingBox::new(min_x, min_y, max_x - min_x, max_y - min_y)
    }
}

/// Rotate a point in the unrotated unit square (bottom-left origin) into the
/// displayed frame for a page `/Rotate` of 0/90/180/270 degrees clockwise.
fn rotate_unit_point(x: f64, y: f64, rotate: u32) -> (f64, f64) {
    match rotate {
        90 => (y, 1.0 - x),
        180 => (1.0 - x, 1.0 - y),
        270 => (1.0 - y, x),
        _ => (x, y),
    }
}

/// A page's boxes and how it is displayed, every inherited value resolved.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PageBoxes {
    /// The media box, `[x0, y0, x1, y1]` with low corners first.
    pub(crate) media: [f64; 4],
    /// The visible box: the crop box clipped to the media box.
    pub(crate) crop: [f64; 4],
    /// The display rotation: 0, 90, 180, or 270.
    pub(crate) rotate: u32,
    /// The size of one user-space unit, in points.
    pub(crate) unit: f64,
    /// The print production boxes the page sets itself (`/BleedBox`,
    /// `/TrimBox`, `/ArtBox`), each clipped to the media box.
    pub(crate) print: [Option<[f64; 4]>; 3],
}

/// The keys of the print production boxes, in [`PageBoxes::print`] order.
pub(crate) const PRINT_BOXES: [&[u8]; 3] = [b"BleedBox", b"TrimBox", b"ArtBox"];

impl PageBoxes {
    /// The boxes of a page with none of its own, as a viewer draws it: US
    /// Letter, unrotated.
    pub(crate) const LETTER: Self = Self {
        media: [0.0, 0.0, 612.0, 792.0],
        crop: [0.0, 0.0, 612.0, 792.0],
        rotate: 0,
        unit: 1.0,
        print: [None; 3],
    };

    /// The page's size as displayed, in points: its visible box turned by its
    /// rotation, at its user unit.
    #[cfg(feature = "render")]
    pub(crate) fn displayed_size(&self) -> (f64, f64) {
        let [x0, y0, x1, y1] = self.crop;
        let (width, height) = ((x1 - x0) * self.unit, (y1 - y0) * self.unit);
        match self.rotate {
            90 | 270 => (height, width),
            _ => (width, height),
        }
    }
}

/// Read a page's boxes, rotation, and user unit, or `None` when the page has
/// no readable box.
///
/// The visible box is the `/CropBox` clipped to the `/MediaBox`, either one
/// alone when the other is missing (or they do not overlap). The boxes and
/// `/Rotate` are inheritable, so this walks up `/Parent` for a value the page
/// itself does not set, resolving references. A missing or junk rotation is
/// zero, and a missing or non-positive unit is one point.
pub(crate) fn page_boxes(doc: &Document, page_id: ObjectId) -> Option<PageBoxes> {
    let media = inherited_rect(doc, page_id, b"MediaBox").map(normalize_rect);
    let crop = inherited_rect(doc, page_id, b"CropBox").map(normalize_rect);
    let (media, crop) = match (media, crop) {
        (Some(media), Some(crop)) => (media, intersect(crop, media).unwrap_or(media)),
        (Some(rect), None) | (None, Some(rect)) => (rect, rect),
        (None, None) => return None,
    };
    let rotate = doc
        .inherited(page_id, b"Rotate", |value| {
            doc.resolve(value)?.as_i64().ok()
        })
        .unwrap_or(0);
    // Normalize rotation to 0/90/180/270 (PDF requires a multiple of 90, but be
    // defensive against junk).
    let rotate = (rotate.rem_euclid(360) as u32 / 90) * 90;
    let unit = doc
        .get_dictionary(page_id)
        .ok()
        .and_then(|page| page.get(b"UserUnit").ok())
        .and_then(|unit| doc.resolve_number(unit))
        .filter(|unit| *unit > 0.0)
        .unwrap_or(1.0);
    let print = PRINT_BOXES.map(|key| {
        let page = doc.get_dictionary(page_id).ok()?;
        intersect(
            normalize_rect(doc.resolve_numbers(page.get(key).ok()?)?),
            media,
        )
    });
    Some(PageBoxes {
        media,
        crop,
        rotate,
        unit,
        print,
    })
}

/// Read a page's visible rectangle and display rotation into a [`PageFrame`],
/// or `None` when the page has no readable box (see [`page_boxes`]).
pub(crate) fn page_frame(doc: &Document, page_id: ObjectId) -> Option<PageFrame> {
    let PageBoxes { crop, rotate, .. } = page_boxes(doc, page_id)?;
    let [x0, y0, x1, y1] = crop;
    Some(PageFrame {
        crop_x0: x0,
        crop_y0: y0,
        crop_w: x1 - x0,
        crop_h: y1 - y0,
        rotate,
    })
}

/// `[x0, y0, x1, y1]` with its corners ordered low to high.
pub(crate) fn normalize_rect([x0, y0, x1, y1]: [f64; 4]) -> [f64; 4] {
    [x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1)]
}

/// The overlap of two normalized rectangles, or `None` when they share no
/// area.
fn intersect(a: [f64; 4], b: [f64; 4]) -> Option<[f64; 4]> {
    let overlap = [
        a[0].max(b[0]),
        a[1].max(b[1]),
        a[2].min(b[2]),
        a[3].min(b[3]),
    ];
    (overlap[0] < overlap[2] && overlap[1] < overlap[3]).then_some(overlap)
}

/// Resolve a 4-number rectangle attribute (`MediaBox`/`CropBox`) on a page,
/// walking up `/Parent` for an inherited value. `None` if absent or malformed.
///
/// The attribute value, and each of its four elements, may be an indirect
/// reference (`/MediaBox 3 0 R`, with `3 0 obj [ 0 0 612 792 ]`) — a valid and
/// common encoding — so both are dereferenced before being read as numbers. A
/// box with any element that is not a resolvable number is rejected (returns
/// `None`), so the caller's fallback chain (CropBox → MediaBox → unit frame)
/// stays available rather than accepting a distorted box.
fn inherited_rect(doc: &Document, page_id: ObjectId, key: &[u8]) -> Option<[f64; 4]> {
    doc.inherited(page_id, key, |value| doc.resolve_numbers(value))
}

/// A page's glyph boxes in default user space and the frame that places them
/// on the page as displayed: what a detected span's rectangles and a reviewer's
/// mark are resolved against, whichever reader produced the glyphs.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PageGeometry<'a> {
    pub(crate) rects: &'a [GlyphRect],
    pub(crate) frame: &'a PageFrame,
}

impl<'a> PageGeometry<'a> {
    /// The page-normalized rectangles covering the characters `chars`, one
    /// per line: the boxes of the glyphs that drew them, merged along each
    /// line.
    pub(crate) fn rects_for(&self, chars: Range<usize>) -> Vec<UnitBoundingBox> {
        let mut lines: Vec<DeviceRect> = Vec::new();
        let covering = self
            .rects
            .iter()
            .filter(|g| g.chars.start < chars.end && g.chars.end > chars.start);
        for glyph in covering {
            match lines.last_mut() {
                Some(line) if same_line(line, &glyph.rect) => *line = line.union(glyph.rect),
                _ => lines.push(glyph.rect),
            }
        }
        lines.into_iter().map(|r| self.frame.normalize(r)).collect()
    }

    /// The page-local character range of each glyph whose center falls in any
    /// of `regions` (page-normalized, top-left origin), in draw order.
    pub(crate) fn glyphs_in(
        self,
        regions: &'a [UnitBoundingBox],
    ) -> impl Iterator<Item = Range<usize>> + 'a {
        let frame = self.frame;
        self.rects
            .iter()
            .filter(move |glyph| {
                let rect = frame.normalize(glyph.rect);
                let x = rect.x + rect.width / 2.0;
                let y = rect.y + rect.height / 2.0;
                regions.iter().any(|region| {
                    (region.x..=region.x + region.width).contains(&x)
                        && (region.y..=region.y + region.height).contains(&y)
                })
            })
            .map(|glyph| glyph.chars.clone())
    }
}

/// Whether two device boxes sit on the same text line: their vertical spans
/// overlap by more than half the smaller box's height.
fn same_line(a: &DeviceRect, b: &DeviceRect) -> bool {
    let overlap = a.max_y.min(b.max_y) - a.min_y.max(b.min_y);
    let min_h = (a.max_y - a.min_y)
        .min(b.max_y - b.min_y)
        .max(f64::MIN_POSITIVE);
    overlap > min_h * 0.5
}

#[cfg(test)]
mod tests {
    use lopdf::{Object, dictionary};

    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    /// An unrotated A4-ish page: a device box maps to crop fractions with the Y
    /// axis flipped to a top-left origin.
    #[test]
    fn normalize_unrotated_flips_y() {
        let frame = PageFrame {
            crop_x0: 0.0,
            crop_y0: 0.0,
            crop_w: 100.0,
            crop_h: 200.0,
            rotate: 0,
        };
        // A box near the page bottom-left in PDF space (y up).
        let rect = DeviceRect {
            min_x: 10.0,
            min_y: 20.0,
            max_x: 30.0,
            max_y: 60.0,
        };
        let u = frame.normalize(rect);
        // x: 10/100..30/100. y (top-left): top = 1 - 60/200 = 0.7, height = 40/200.
        assert!(close(u.x, 0.1), "{u:?}");
        assert!(close(u.width, 0.2), "{u:?}");
        assert!(close(u.y, 0.7), "{u:?}");
        assert!(close(u.height, 0.2), "{u:?}");
    }

    /// A non-zero crop origin shifts the box before normalizing.
    #[test]
    fn normalize_respects_crop_origin() {
        let frame = PageFrame {
            crop_x0: 50.0,
            crop_y0: 100.0,
            crop_w: 100.0,
            crop_h: 100.0,
            rotate: 0,
        };
        let rect = DeviceRect {
            min_x: 60.0,
            min_y: 110.0,
            max_x: 70.0,
            max_y: 120.0,
        };
        let u = frame.normalize(rect);
        // x: (60-50)/100 = 0.1. top: 1 - (120-100)/100 = 0.8.
        assert!(close(u.x, 0.1), "{u:?}");
        assert!(close(u.width, 0.1), "{u:?}");
        assert!(close(u.y, 0.8), "{u:?}");
        assert!(close(u.height, 0.1), "{u:?}");
    }

    /// A 90° page rotation maps the unrotated unit square into the displayed
    /// frame: a point at unrotated (x, y) lands at (y, 1 - x) before the Y-flip.
    #[test]
    fn normalize_rotates_90() {
        let frame = PageFrame {
            crop_x0: 0.0,
            crop_y0: 0.0,
            crop_w: 100.0,
            crop_h: 100.0,
            rotate: 90,
        };
        // Unrotated crop fractions: x 0.1..0.2, y 0.3..0.4 (bottom-left origin).
        let rect = DeviceRect {
            min_x: 10.0,
            min_y: 30.0,
            max_x: 20.0,
            max_y: 40.0,
        };
        let u = frame.normalize(rect);
        // rotate_unit_point(x,y,90) = (y, 1-x). Corners (0.1,0.3)->(0.3,0.9),
        // (0.2,0.4)->(0.4,0.8). x: 0.3..0.4. bottom-left y: 0.8..0.9 → top-left
        // (1-y): 0.1..0.2.
        assert!(close(u.x, 0.3), "{u:?}");
        assert!(close(u.width, 0.1), "{u:?}");
        assert!(close(u.y, 0.1), "{u:?}");
        assert!(close(u.height, 0.1), "{u:?}");
    }

    #[test]
    fn inherited_rect_reads_a_direct_array() {
        let mut doc = Document::with_version("1.7");
        let page = doc.add_object(dictionary! {
            "Type" => "Page",
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        });
        assert_eq!(
            inherited_rect(&doc, page, b"MediaBox"),
            Some([0.0, 0.0, 612.0, 792.0]),
        );
    }

    #[test]
    fn inherited_rect_dereferences_an_indirect_box() {
        // `/MediaBox 3 0 R` with `3 0 obj [ 0 0 612 792 ]` — a valid, common
        // encoding that the direct `.as_array()` could not read, so the page
        // frame fell back to a unit square and geometry stayed in raw points.
        let mut doc = Document::with_version("1.7");
        let box_id = doc.add_object(Object::Array(vec![
            0.into(),
            0.into(),
            612.into(),
            792.into(),
        ]));
        let page = doc.add_object(dictionary! {
            "Type" => "Page",
            "MediaBox" => Object::Reference(box_id),
        });
        assert_eq!(
            inherited_rect(&doc, page, b"MediaBox"),
            Some([0.0, 0.0, 612.0, 792.0]),
            "an indirect MediaBox must dereference to its array",
        );
    }

    #[test]
    fn inherited_rect_rejects_a_box_with_an_unresolvable_element() {
        // An element that is a dangling reference must fail the whole box (so the
        // caller falls back to MediaBox), not silently resolve to 0.
        let mut doc = Document::with_version("1.7");
        let dangling = doc.new_object_id(); // never inserted
        let page = doc.add_object(dictionary! {
            "Type" => "Page",
            "CropBox" => vec![
                0.into(),
                0.into(),
                Object::Reference(dangling),
                792.into(),
            ],
        });
        assert_eq!(inherited_rect(&doc, page, b"CropBox"), None);
    }

    #[test]
    fn inherited_rect_rejects_a_non_numeric_element() {
        let mut doc = Document::with_version("1.7");
        let page = doc.add_object(dictionary! {
            "Type" => "Page",
            "CropBox" => vec![
                0.into(),
                0.into(),
                Object::Name(b"oops".to_vec()),
                792.into(),
            ],
        });
        assert_eq!(inherited_rect(&doc, page, b"CropBox"), None);
    }

    #[test]
    fn inherited_rect_walks_up_to_an_inherited_box() {
        let mut doc = Document::with_version("1.7");
        let pages = doc.new_object_id();
        let page = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages,
        });
        doc.objects.insert(
            pages,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
            }),
        );
        assert_eq!(
            inherited_rect(&doc, page, b"MediaBox"),
            Some([0.0, 0.0, 595.0, 842.0]),
        );
    }

    /// A page with `boxes` set on it directly, and its id.
    fn page_with(boxes: Vec<(&str, Object)>) -> (Document, ObjectId) {
        let mut doc = Document::with_version("1.7");
        let mut page = dictionary! { "Type" => "Page" };
        for (key, value) in boxes {
            page.set(key, value);
        }
        let id = doc.add_object(page);
        (doc, id)
    }

    fn rect(values: [i64; 4]) -> Object {
        Object::Array(values.into_iter().map(Object::Integer).collect())
    }

    /// The visible page is the crop box clipped to the media box.
    #[test]
    fn the_crop_box_is_clipped_to_the_media_box() {
        let (doc, page) = page_with(vec![
            ("MediaBox", rect([0, 0, 600, 800])),
            ("CropBox", rect([-100, 0, 600, 800])),
        ]);
        let frame = page_frame(&doc, page).unwrap();
        assert!(
            close(frame.crop_x0, 0.0) && close(frame.crop_w, 600.0),
            "{frame:?}"
        );
    }

    #[test]
    fn an_indirect_rotation_is_read() {
        let (mut doc, page) = page_with(vec![("MediaBox", rect([0, 0, 600, 800]))]);
        let rotation = doc.add_object(Object::Integer(90));
        doc.get_dictionary_mut(page)
            .unwrap()
            .set("Rotate", rotation);
        assert_eq!(page_frame(&doc, page).unwrap().rotate, 90);
    }
}
