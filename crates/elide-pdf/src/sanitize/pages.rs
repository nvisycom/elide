//! The page tree and each page: its boxes, rotation, and content copied, and
//! what the source's page-tree nodes held recorded.

use std::collections::HashSet;

use elide_core::redaction::HiddenContent;
use elide_core::{Error, Result};
use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Object, ObjectId, dictionary};

use super::content::Fresh;
use super::copy::At;
use super::keys::Kind;
use super::widgets::Flattened;
use super::{Builder, content, record};
use crate::pdf::reader::{DictionaryExt, DocumentExt, PRINT_BOXES, PageBoxes, Scope, page_boxes};
use crate::pdf::writer::{compressed, encoded};

impl<'a> Builder<'a> {
    /// Record the entries of the page `id` and of each page-tree node above it
    /// not yet in `nodes` that the rebuild does not carry.
    pub(super) fn record_page_tree(&mut self, id: ObjectId, nodes: &mut HashSet<ObjectId>) {
        let source = self.source;
        let mut current = Some(id);
        while let Some(node) = current.take() {
            if !nodes.insert(node) {
                break;
            }
            let Ok(dict) = source.get_dictionary(node) else {
                break;
            };
            for (key, _) in dict {
                if let Some(held) = record::page(key) {
                    self.record(held);
                }
            }
            current = dict.get(b"Parent").and_then(Object::as_reference).ok();
        }
    }

    /// Copy the page `id` under the page-tree node `parent`, returning its
    /// copy's id.
    pub(super) fn page(&mut self, id: ObjectId, parent: ObjectId) -> Result<ObjectId> {
        let source = self.source;
        let copy = self.pages[&id];
        let dict = source
            .get_dictionary(id)
            .map_err(|e| Error::malformed(format!("page {id:?}: {e}")))?;
        let mut page = dictionary! { "Type" => "Page", "Parent" => parent };
        // A page with no box of its own or inherited is drawn as US Letter.
        let boxes = page_boxes(source, id).unwrap_or(PageBoxes::LETTER);
        self.boxes(dict, &boxes, &mut page);

        let scope = Scope::page(source, id);
        let bytes = source.page_content(id)?;
        let mut resources = Dictionary::new();
        let mut content = self.content(&bytes, At::top(&scope), &mut resources)?;

        // A form field is flattened into the page, drawn after its content in
        // the initial graphics state (the content is balanced, so a `q … Q`
        // around it restores that state).
        let (links, appearances) = self.annotations(dict, &scope, boxes.rotate)?;
        if !links.is_empty() {
            page.set("Annots", Object::Array(links));
        }
        if !appearances.is_empty() {
            let drawn = drawn(appearances, &mut resources)?;
            content = [b"q\n".as_slice(), &content, b"\nQ\n", &drawn].concat();
        }

        self.charge(content.len())?;
        page.set(
            "Contents",
            self.out.add_object(compressed(Dictionary::new(), content)?),
        );
        page.set("Resources", resources);

        let own: [(&[u8], Kind); 2] = [
            (b"Group", Kind::Group),
            (b"OutputIntents", Kind::OutputIntent),
        ];
        for (key, kind) in own {
            if let Ok(value) = dict.get(key)
                && let Some(value) = self.copy(kind, value, At::top(&scope))?
            {
                page.set(key, value);
            }
        }
        if let Some(af) = dict
            .get(b"AF")
            .ok()
            .and_then(|af| self.associated(af).transpose())
        {
            page.set("AF", af?);
        }
        self.out.objects.insert(copy, Object::Dictionary(page));
        Ok(copy)
    }

    /// The annotations of the page `dict`, drawn in `scope` at rotation
    /// `rotate`: the internal links kept, and the form fields flattened.
    /// Anything else is left behind, recorded as what it was.
    ///
    /// # Errors
    ///
    /// As [`link`](Self::link) and [`flatten`](Self::flatten).
    fn annotations(
        &mut self,
        dict: &'a Dictionary,
        scope: &Scope<'a>,
        rotate: u32,
    ) -> Result<(Vec<Object>, Vec<Flattened>)> {
        let source = self.source;
        let mut links = Vec::new();
        let mut appearances = Vec::new();
        let annotations = dict.resolved_array(source, b"Annots");
        for annotation in annotations.into_iter().flatten() {
            let Some(annotation) = source.resolve_dict(annotation) else {
                self.residue += 1;
                continue;
            };
            if annotation.is_subtype(source, b"Widget") {
                appearances.extend(self.flatten(annotation, scope, rotate)?);
            } else if let Some(link) = self.link(annotation)? {
                links.push(link);
            }
        }
        Ok((links, appearances))
    }

    /// Set `boxes`, the boxes and rotation of the page `dict`, on its copy
    /// `page`. The media box is required, so always written; the others only
    /// where they differ from the default. A print box the page sets that is
    /// not a box within its media box is not written, and is recorded as a
    /// property dropped.
    fn boxes(&mut self, dict: &Dictionary, boxes: &PageBoxes, page: &mut Dictionary) {
        page.set("MediaBox", rect(boxes.media));
        if boxes.crop != boxes.media {
            page.set("CropBox", rect(boxes.crop));
        }
        if boxes.rotate != 0 {
            page.set("Rotate", i64::from(boxes.rotate));
        }
        if boxes.unit != 1.0 {
            page.set("UserUnit", Object::Real(boxes.unit as f32));
        }
        for (key, print) in PRINT_BOXES.into_iter().zip(boxes.print) {
            match print {
                Some(print) => page.set(key, rect(print)),
                None if dict.get(key).is_ok() => self.record(HiddenContent::Properties),
                None => {}
            }
        }
    }
}

/// The content drawing the flattened form fields `appearances`, each its
/// form under its graphics state and matrix, inside `q … Q`; their forms and
/// graphics states are added to `resources` under fresh names.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if
/// the content can't be encoded.
fn drawn(appearances: Vec<Flattened>, resources: &mut Dictionary) -> Result<Vec<u8>> {
    let mut drawn = Vec::with_capacity(appearances.len() * 5);
    let (mut forms, mut states) = (Fresh::new("Annot"), Fresh::new("AnnotGS"));
    for flattened in appearances {
        let name = forms.name(|name| content::has(resources, b"XObject", name));
        content::insert(resources, b"XObject", &name, flattened.form);
        drawn.push(Operation::new("q", Vec::new()));
        if let Some(state) = flattened.state {
            let state_name = states.name(|name| content::has(resources, b"ExtGState", name));
            content::insert(
                resources,
                b"ExtGState",
                &state_name,
                Object::Dictionary(state),
            );
            drawn.push(Operation::new("gs", vec![Object::Name(state_name)]));
        }
        let matrix = flattened.matrix.values().map(|v| Object::Real(v as f32));
        drawn.push(Operation::new("cm", matrix.to_vec()));
        drawn.push(Operation::new("Do", vec![Object::Name(name)]));
        drawn.push(Operation::new("Q", Vec::new()));
    }
    encoded(&Content { operations: drawn })
}

/// `[x0, y0, x1, y1]` as a PDF rectangle.
fn rect(corners: [f64; 4]) -> Object {
    Object::Array(corners.map(|v| Object::Real(v as f32)).to_vec())
}
