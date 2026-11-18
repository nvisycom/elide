#!/usr/bin/env python3
"""Generate `crates/elide-office/src/opc/schema/table.rs` from the OOXML schemas.

The office rebuild copies an element only if the schema allows it under its
parent, keeping only the attributes its type declares. This script reads the
schemas once and writes that allowlist as a Rust table: every element type, the
child elements (each with its own type) and attributes it allows, and whether
it holds text.

Usage:

    scripts/gen-ooxml-schema.py <schemas>

where `<schemas>` holds:

- `ISO-IEC29500-4_2016/`: the transitional schemas of ISO/IEC 29500-4:2016
  (ECMA-376 Part 4), `wml.xsd`, `sml.xsd`, `pml.xsd`, `dml-*.xsd`,
  `vml-*.xsd`, and `shared-*.xsd`;
- `microsoft/`: the WordprocessingML extension schemas of [MS-DOCX]
  (`wml-2010.xsd`, `wml-2012.xsd`, ...), whose parts (comment extensions,
  people) have roots of their own.

Markup compatibility (`mc:AlternateContent`) is resolved by the rebuild itself,
so its schema is not read. A wildcard (`xsd:any`) that names its namespaces
(`w:pict` admits VML) allows the global elements of those namespaces; one open
to any namespace allows nothing, what it would admit being outside the schema,
except `a:graphicData`, whose standard payloads (`PAYLOADS`) are listed here.
`xml:space` is allowed on every element that holds text: the schemas leave it
off SpreadsheetML's strings, where Excel writes it.
"""

import sys
import xml.etree.ElementTree as ET
from pathlib import Path

XSD = "{http://www.w3.org/2001/XMLSchema}"
MCE = "http://schemas.openxmlformats.org/markup-compatibility/2006"
XML = "http://www.w3.org/XML/1998/namespace"
A = "http://schemas.openxmlformats.org/drawingml/2006/main"
OUT = Path(__file__).resolve().parent.parent / "crates/elide-office/src/opc/schema/table.rs"

# The standard payloads of `a:graphicData`, whose schema admits any element:
# a picture, chart, SmartArt diagram, locked canvas, table, and OLE object.
PAYLOADS = [
    ("http://schemas.openxmlformats.org/drawingml/2006/picture", "pic"),
    ("http://schemas.openxmlformats.org/drawingml/2006/chart", "chart"),
    ("http://schemas.openxmlformats.org/drawingml/2006/diagram", "relIds"),
    ("http://schemas.openxmlformats.org/drawingml/2006/lockedCanvas", "lockedCanvas"),
    (A, "tbl"),
    ("http://schemas.openxmlformats.org/presentationml/2006/main", "oleObj"),
]

# Roots the Rust dispatch names, which a complete schema set defines.
REQUIRED_ROOTS = [
    ("http://schemas.openxmlformats.org/wordprocessingml/2006/main", "document"),
    ("http://schemas.openxmlformats.org/presentationml/2006/main", "presentation"),
    ("http://schemas.openxmlformats.org/spreadsheetml/2006/main", "workbook"),
    (A, "theme"),
    ("urn:schemas-microsoft-com:vml", "shape"),
    ("http://schemas.microsoft.com/office/word/2012/wordml", "commentsEx"),
    ("http://schemas.microsoft.com/office/word/2012/wordml", "people"),
    ("http://schemas.microsoft.com/office/word/2016/wordml/cid", "commentsIds"),
    ("http://schemas.microsoft.com/office/word/2018/wordml/cex", "commentsExtensible"),
]


class Schema:
    """One schema document: its target namespace, prefixes, and form defaults."""

    def __init__(self, path):
        self.path = path
        # ElementTree drops namespace declarations, so read them first.
        # The `xml` prefix is bound without a declaration.
        self.prefixes = {"xml": XML}
        for _, (prefix, uri) in ET.iterparse(path, events=["start-ns"]):
            self.prefixes.setdefault(prefix, uri)
        self.root = ET.parse(path).getroot()
        self.tns = self.root.get("targetNamespace", "")
        self.elements_qualified = self.root.get("elementFormDefault") == "qualified"
        self.attributes_qualified = self.root.get("attributeFormDefault") == "qualified"

    def qname(self, value):
        """The (namespace, local) a QName attribute value names."""
        prefix, _, local = value.rpartition(":")
        return (self.prefixes.get(prefix, ""), local)


class Generator:
    def __init__(self, schemas):
        self.schemas = schemas
        # Global declarations, by (namespace, name): (schema, node).
        self.complex = {}
        self.simple = {}
        self.elements = {}
        self.groups = {}
        self.attribute_groups = {}
        # Global elements substituting for another, by the head's name.
        self.substitutes = {}
        for schema in schemas:
            for node in schema.root:
                name = node.get("name")
                key = (schema.tns, name)
                tag = node.tag
                if tag == XSD + "complexType":
                    self.complex[key] = (schema, node)
                elif tag == XSD + "simpleType":
                    self.simple[key] = (schema, node)
                elif tag == XSD + "element":
                    self.elements[key] = (schema, node)
                    head = node.get("substitutionGroup")
                    if head:
                        self.substitutes.setdefault(schema.qname(head), []).append(key)
                elif tag == XSD + "group":
                    self.groups[key] = (schema, node)
                elif tag == XSD + "attributeGroup":
                    self.attribute_groups[key] = (schema, node)
        # Resolved types, by key: {"text", "wildcard", "attributes", "children"}.
        self.types = {}
        self.anonymous = 0

    # Types -----------------------------------------------------------------

    def type_of_element(self, schema, node):
        """The type key of the element declaration `node`."""
        inline = node.find(XSD + "complexType")
        if inline is not None:
            self.anonymous += 1
            key = ("anon", self.anonymous)
            self.resolve(key, schema, inline)
            return key
        value = node.get("type")
        if value is None:
            return "text"
        name = schema.qname(value)
        if name in self.complex:
            key = ("ct",) + name
            if key not in self.types:
                owner, ct = self.complex[name]
                self.resolve(key, owner, ct)
            return key
        return "text"

    def resolve(self, key, schema, node):
        """Resolve the complex type `node` into `self.types[key]`."""
        resolved = {"text": False, "wildcard": False, "attributes": set(), "children": {}}
        # Registered first, so a type that nests itself resolves.
        self.types[key] = resolved
        if node.get("mixed") == "true":
            resolved["text"] = True
        self.content(schema, node, resolved)

    def merge_base(self, schema, base, resolved, children=True):
        """Merge the type named `base` into `resolved`."""
        name = schema.qname(base)
        if name in self.complex:
            key = ("ct",) + name
            if key not in self.types:
                owner, ct = self.complex[name]
                self.resolve(key, owner, ct)
            other = self.types[key]
            resolved["attributes"] |= other["attributes"]
            if children:
                resolved["children"].update(other["children"])
                resolved["wildcard"] |= other["wildcard"]
            resolved["text"] |= other["text"]
        else:
            # A simple type, or a builtin.
            resolved["text"] = True

    def content(self, schema, node, resolved):
        for child in node:
            tag = child.tag
            if tag in (XSD + "sequence", XSD + "choice", XSD + "all"):
                self.particles(schema, child, resolved)
            elif tag == XSD + "group":
                self.particles(schema, child, resolved)
            elif tag in (XSD + "attribute", XSD + "attributeGroup", XSD + "anyAttribute"):
                self.attributes(schema, child, resolved)
            elif tag == XSD + "complexContent":
                if child.get("mixed") == "true":
                    resolved["text"] = True
                for derivation in child:
                    if derivation.tag == XSD + "extension":
                        self.merge_base(schema, derivation.get("base"), resolved)
                    elif derivation.tag == XSD + "restriction":
                        # A restriction restates the content; its attributes
                        # are inherited.
                        self.merge_base(schema, derivation.get("base"), resolved, children=False)
                    self.content(schema, derivation, resolved)
            elif tag == XSD + "simpleContent":
                resolved["text"] = True
                for derivation in child:
                    if derivation.tag in (XSD + "extension", XSD + "restriction"):
                        self.merge_base(schema, derivation.get("base"), resolved, children=False)
                        self.content(schema, derivation, resolved)

    def particles(self, schema, node, resolved):
        tag = node.tag
        if tag == XSD + "group" and node.get("ref"):
            owner, group = self.groups[schema.qname(node.get("ref"))]
            for child in group:
                self.particles(owner, child, resolved)
            return
        if tag in (XSD + "sequence", XSD + "choice", XSD + "all", XSD + "group"):
            for child in node:
                self.particles(schema, child, resolved)
        elif tag == XSD + "any":
            namespaces = node.get("namespace", "##any").split()
            named = [ns for ns in namespaces if not ns.startswith("##")]
            if named and len(named) == len(namespaces):
                # A wildcard naming its namespaces admits their global
                # elements, each read against its own type.
                for key, (owner, element) in sorted(self.elements.items()):
                    if key[0] in named:
                        resolved["children"][key] = self.type_of_element(owner, element)
            else:
                resolved["wildcard"] = True
        elif tag == XSD + "element":
            if node.get("ref"):
                name = schema.qname(node.get("ref"))
                if name[0] == MCE:
                    return
                for key in [name] + self.substitutes.get(name, []):
                    owner, element = self.elements[key]
                    resolved["children"][key] = self.type_of_element(owner, element)
            else:
                form = node.get("form")
                qualified = form == "qualified" if form else schema.elements_qualified
                name = (schema.tns if qualified else "", node.get("name"))
                resolved["children"][name] = self.type_of_element(schema, node)

    def attributes(self, schema, node, resolved):
        tag = node.tag
        if tag == XSD + "attribute":
            if node.get("ref"):
                name = schema.qname(node.get("ref"))
                if name[0] == MCE:
                    return
            else:
                form = node.get("form")
                qualified = form == "qualified" if form else schema.attributes_qualified
                name = (schema.tns if qualified else "", node.get("name"))
            if node.get("use") != "prohibited":
                resolved["attributes"].add(name)
        elif tag == XSD + "attributeGroup":
            owner, group = self.attribute_groups[schema.qname(node.get("ref"))]
            for child in group:
                self.attributes(owner, child, resolved)

    # Output ----------------------------------------------------------------

    def run(self):
        roots = {}
        for name, (schema, node) in sorted(self.elements.items()):
            if name[0] == MCE:
                continue
            roots[name] = self.type_of_element(schema, node)
        missing = [f"{{{ns}}}{local}" for ns, local in REQUIRED_ROOTS if (ns, local) not in roots]
        if missing:
            sys.exit(f"incomplete schema set: no root {', '.join(missing)}")

        # The payloads `a:graphicData` holds.
        graphic_data = self.types[("ct", A, "CT_GraphicalObjectData")]
        for name in PAYLOADS:
            graphic_data["children"][name] = roots[name]
        graphic_data["wildcard"] = False

        # `xml:space` on every element holding text.
        for t in self.types.values():
            if t["text"]:
                t["attributes"].add((XML, "space"))

        # Number the types, folding identical ones together; type 0 is plain
        # text.
        namespaces = sorted({ns for ns, _ in roots} | {
            ns
            for t in self.types.values()
            for ns, _ in list(t["attributes"]) + list(t["children"])
        })
        if "" in namespaces:
            namespaces.remove("")
        namespaces = [""] + namespaces
        ns_index = {ns: i for i, ns in enumerate(namespaces)}

        signature = {}
        ids = {"text": 0}
        bodies = [(True, False, ((ns_index[XML], "space"),), ())]

        def number(key):
            if key in ids:
                return ids[key]
            # A type that nests itself is numbered before its children are.
            ids[key] = len(bodies)
            bodies.append(None)
            t = self.types[key]
            attributes = tuple(sorted((ns_index[ns], local) for ns, local in t["attributes"]))
            children = tuple(sorted(
                (ns_index[ns], local, number(child))
                for (ns, local), child in t["children"].items()
            ))
            bodies[ids[key]] = (t["text"], t["wildcard"], attributes, children)
            return ids[key]

        root_entries = sorted(
            (ns_index[ns], local, number(key)) for (ns, local), key in roots.items()
        )
        # Fold identical types together by partition refinement: types that
        # agree on their text, attributes, and child names, and whose children
        # fold together, are one.
        def renumber(signatures):
            seen = {}
            return [seen.setdefault(sig, len(seen)) for sig in signatures]

        remap = renumber(
            (text, wildcard, attributes, tuple((n, l) for n, l, _ in children))
            for text, wildcard, attributes, children in bodies
        )
        while True:
            refined = renumber(
                (remap[i], tuple(remap[c] for _, _, c in body[3]))
                for i, body in enumerate(bodies)
            )
            if len(set(refined)) == len(set(remap)):
                break
            remap = refined
        final = {}
        for i, (text, wildcard, attributes, children) in enumerate(bodies):
            final[remap[i]] = (
                text,
                wildcard,
                attributes,
                tuple((n, l, remap[c]) for n, l, c in children),
            )
        root_entries = sorted((n, l, remap[c]) for n, l, c in root_entries)
        self.write(namespaces, [final[i] for i in range(len(final))], root_entries)

    def write(self, namespaces, types, roots):
        out = []
        w = out.append
        w("// @generated by scripts/gen-ooxml-schema.py from the ISO/IEC 29500-4:2016")
        w("// transitional schemas and the [MS-DOCX] extension schemas. Do not edit.")
        w("")
        w("use super::{Child, Type};")
        w("")
        w("/// The namespaces the schemas name, by index; 0 is no namespace.")
        w(f"pub(super) static NAMESPACES: [&str; {len(namespaces)}] = [")
        for ns in namespaces:
            w(f'    "{ns}",')
        w("];")
        w("")
        w("/// Every element type, by index; 0 holds text alone, with `xml:space`.")
        w(f"pub(super) static TYPES: [Type; {len(types)}] = [")
        for text, wildcard, attributes, children in types:
            attrs = ", ".join(f'({n}, "{l}")' for n, l in attributes)
            kids = ", ".join(f'Child({n}, "{l}", {c})' for n, l, c in children)
            w(
                f"    Type {{ text: {str(text).lower()}, wildcard: {str(wildcard).lower()}, "
                f"attributes: &[{attrs}], children: &[{kids}] }},"
            )
        w("];")
        w("")
        w("/// The elements a part may have as its root, with their types.")
        w(f"pub(super) static ROOTS: [Child; {len(roots)}] = [")
        for n, l, c in roots:
            w(f'    Child({n}, "{l}", {c}),')
        w("];")
        OUT.write_text("\n".join(out) + "\n")
        print(f"{OUT}: {len(namespaces)} namespaces, {len(types)} types, {len(roots)} roots")


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    base = Path(sys.argv[1])
    paths = []
    for directory in ("ISO-IEC29500-4_2016", "microsoft"):
        found = sorted((base / directory).glob("*.xsd"))
        if not found:
            sys.exit(f"no schemas under {base / directory}")
        paths += found
    Generator([Schema(path) for path in paths]).run()


if __name__ == "__main__":
    main()
