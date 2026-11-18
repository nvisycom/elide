//! The page tree and each page: its boxes, rotation, and content copied, and
//! what the source's page-tree nodes held recorded.

use std::collections::HashSet;

use elide_core::{Error, Result};
use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Object, ObjectId};

use super::copy::At;
use super::keys::Kind;
use super::{Builder, content, record};
use crate::pdf::reader::{DictionaryExt, DocumentExt, PRINT_BOXES, PageBoxes, Scope, page_boxes};
use crate::pdf::writer::compressed;

impl Builder<'_> {
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
            for (key, _) in dict.iter() {
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
        let mut page = Dictionary::from_iter([
            ("Type", Object::Name(b"Page".to_vec())),
            ("Parent", Object::Reference(parent)),
        ]);
        // A page with no box of its own or inherited is drawn as US Letter,
        // and the media box is required, so it is written.
        let boxes = page_boxes(source, id).unwrap_or(PageBoxes::LETTER);
        {
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
                if let Some(print) = print {
                    page.set(key, rect(print));
                }
            }
        }

        let dict = source
            .get_dictionary(id)
            .map_err(|e| Error::malformed(format!("page {id:?}: {e}")))?;
        let scope = Scope::page(source, id);
        let bytes = source.page_content(id)?;
        let mut resources = Dictionary::new();
        let mut content = self.content(&bytes, At::top(&scope), &mut resources)?;

        // Internal links are kept; a form field is flattened into the page,
        // drawn after its content in the initial graphics state (the content
        // is balanced, so a `q … Q` around it restores that state).
        let mut links = Vec::new();
        let mut appearances = Vec::new();
        let annotations = dict
            .get(b"Annots")
            .ok()
            .and_then(|annotations| source.resolve_array(annotations));
        for annotation in annotations.into_iter().flatten() {
            let Some(annotation) = source.resolve_dict(annotation) else {
                self.residue += 1;
                continue;
            };
            if annotation.is_subtype(self.source, b"Widget") {
                appearances.extend(self.flatten(annotation, &scope, boxes.rotate)?);
                continue;
            }
            if let Some(link) = self.link(annotation)? {
                links.push(link);
            }
        }
        if !links.is_empty() {
            page.set("Annots", Object::Array(links));
        }
        if !appearances.is_empty() {
            let mut drawn = Vec::with_capacity(appearances.len() * 5);
            for flattened in appearances {
                let name =
                    content::fresh("Annot", |name| content::has(&resources, b"XObject", name));
                content::insert(&mut resources, b"XObject", &name, flattened.form);
                drawn.push(Operation::new("q", Vec::new()));
                if let Some(state) = flattened.state {
                    let state_name = content::fresh("AnnotGS", |name| {
                        content::has(&resources, b"ExtGState", name)
                    });
                    content::insert(
                        &mut resources,
                        b"ExtGState",
                        &state_name,
                        Object::Dictionary(state),
                    );
                    drawn.push(Operation::new("gs", vec![Object::Name(state_name)]));
                }
                drawn.push(Operation::new(
                    "cm",
                    flattened
                        .matrix
                        .values()
                        .map(|v| Object::Real(v as f32))
                        .to_vec(),
                ));
                drawn.push(Operation::new("Do", vec![Object::Name(name)]));
                drawn.push(Operation::new("Q", Vec::new()));
            }
            let drawn = Content { operations: drawn }
                .encode()
                .map_err(|e| Error::malformed(format!("encode content: {e}")))?;
            content = [b"q\n".as_slice(), &content, b"\nQ\n", &drawn].concat();
        }

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
}

/// `[x0, y0, x1, y1]` as a PDF rectangle.
fn rect(corners: [f64; 4]) -> Object {
    Object::Array(corners.map(|v| Object::Real(v as f32)).to_vec())
}
