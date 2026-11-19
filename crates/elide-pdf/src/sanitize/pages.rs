//! The page tree and each page: its boxes, rotation, and content copied, and
//! what the source's page-tree nodes held recorded.

use std::collections::HashSet;

use elide_core::Result;
use elide_core::redaction::HiddenContent;
use hayro_syntax::object::{Array, Dict, Object};

use super::content::Fresh;
use super::copy::At;
use super::keys::Kind;
use super::{Builder, content, record};
use crate::pdf::content::{Operation, encode};
use crate::pdf::object::{Dictionary, Id, Value};
use crate::pdf::reader::{
    DictExt, ObjectId, PRINT_BOXES, PageBoxes, Scope, page_boxes, page_content,
};
use crate::pdf::writer::compressed;

impl<'a> Builder<'a> {
    /// Record the entries of the page `page` and of each page-tree node
    /// above it not yet in `nodes` that the rebuild does not carry.
    pub(super) fn record_page_tree(&mut self, page: &Dict<'a>, nodes: &mut HashSet<ObjectId>) {
        let mut current = Some(page.clone());
        while let Some(dict) = current.take() {
            let Some(node) = dict.obj_id() else {
                break;
            };
            if !nodes.insert(node) {
                break;
            }
            for key in dict.keys() {
                if let Some(held) = record::page(&key) {
                    self.record(held);
                }
            }
            current = dict.get::<Dict<'a>>(b"Parent");
        }
    }

    /// Copy the page `dict` (the object `id`) under the page-tree node
    /// `parent`, returning its copy's id.
    pub(super) fn page(&mut self, id: ObjectId, dict: &Dict<'a>, parent: Id) -> Result<Id> {
        let copy = self.pages[&id];
        let mut page = Dictionary::from_iter([
            ("Type", Value::name("Page")),
            ("Parent", Value::Ref(parent)),
        ]);
        // A page with no box of its own or inherited is drawn as US Letter,
        // and the media box is required, so it is written.
        let boxes = page_boxes(dict).unwrap_or(PageBoxes::LETTER);
        {
            page.set("MediaBox", rect(boxes.media));
            if boxes.crop != boxes.media {
                page.set("CropBox", rect(boxes.crop));
            }
            if boxes.rotate != 0 {
                page.set("Rotate", boxes.rotate);
            }
            if boxes.unit != 1.0 {
                page.set("UserUnit", boxes.unit);
            }
            // A print box the page sets that is not a box within its media
            // box is not written, and is recorded as a property dropped.
            for (key, print) in PRINT_BOXES.into_iter().zip(boxes.print) {
                match print {
                    Some(print) => page.set(key, rect(print)),
                    None if dict.contains_key(key) => {
                        self.record(HiddenContent::Properties);
                    }
                    None => {}
                }
            }
        }

        let scope = Scope::page(dict);
        let bytes = page_content(dict)?;
        let mut resources = Dictionary::new();
        let mut content = self.content(&bytes, At::top(&scope), &mut resources)?;

        // Internal links are kept; a form field is flattened into the page,
        // drawn after its content in the initial graphics state (the content
        // is balanced, so a `q … Q` around it restores that state).
        let mut links = Vec::new();
        let mut appearances = Vec::new();
        let annotations = dict.get::<Array<'a>>(b"Annots");
        for annotation in annotations.iter().flat_map(|a| a.iter::<Object<'a>>()) {
            let Some(annotation) = annotation.into_dict() else {
                self.residue += 1;
                continue;
            };
            if annotation.is_subtype(b"Widget") {
                appearances.extend(self.flatten(&annotation, &scope, boxes.rotate)?);
                continue;
            }
            if let Some(link) = self.link(&annotation)? {
                links.push(link);
            }
        }
        if !links.is_empty() {
            page.set("Annots", Value::Array(links));
        }
        if !appearances.is_empty() {
            let mut drawn = Vec::with_capacity(appearances.len() * 5);
            let (mut forms, mut states) = (Fresh::new("Annot"), Fresh::new("AnnotGS"));
            for flattened in appearances {
                let name = forms.name(|name| content::has(&resources, b"XObject", name));
                content::insert(&mut resources, b"XObject", &name, flattened.form);
                drawn.push(Operation::new("q", Vec::new()));
                if let Some(state) = flattened.state {
                    let state_name =
                        states.name(|name| content::has(&resources, b"ExtGState", name));
                    content::insert(
                        &mut resources,
                        b"ExtGState",
                        &state_name,
                        Value::Dict(state),
                    );
                    drawn.push(Operation::new("gs", vec![Value::Name(state_name)]));
                }
                drawn.push(Operation::new(
                    "cm",
                    flattened.matrix.values().map(Value::Real).to_vec(),
                ));
                drawn.push(Operation::new("Do", vec![Value::Name(name)]));
                drawn.push(Operation::new("Q", Vec::new()));
            }
            let drawn = encode(&drawn)?;
            content = [b"q\n".as_slice(), &content, b"\nQ\n", &drawn].concat();
        }

        self.charge(content.len())?;
        page.set(
            "Contents",
            self.out.add(compressed(Dictionary::new(), &content)?),
        );
        page.set("Resources", resources);

        let own: [(&[u8], Kind); 2] = [
            (b"Group", Kind::Group),
            (b"OutputIntents", Kind::OutputIntent),
        ];
        for (key, kind) in own {
            if let Some(value) = dict.raw(key)
                && let Some(value) = self.copy(kind, &value, At::top(&scope))?
            {
                page.set(key, value);
            }
        }
        if let Some(af) = dict.raw(b"AF")
            && let Some(af) = self.associated(&af)?
        {
            page.set("AF", af);
        }
        self.out.set(copy, page);
        Ok(copy)
    }
}

/// `[x0, y0, x1, y1]` as a PDF rectangle.
fn rect(corners: [f64; 4]) -> Value {
    Value::Array(corners.map(Value::Real).to_vec())
}
