#!/usr/bin/env python3
"""Fetch the OOXML schemas `scripts/office/gen-ooxml-schema.py` reads.

Usage:

    scripts/office/fetch-ooxml-schemas.py <schemas>
    scripts/office/gen-ooxml-schema.py <schemas>

Every source is pinned by URL and SHA-256 and refused on a mismatch, so the
same `<schemas>` directory, and from it the same table, comes back every time:

- `ecma-376-4/`: the transitional schemas of ECMA-376 Part 4, 5th
  edition, the text ISO/IEC 29500-4:2016 adopts. ISO's own download sits
  behind a licence page; Ecma's is direct. They are unpacked unmodified.
- `microsoft/`: the extension schemas of [MS-DOCX], [MS-ODRAWXML],
  [MS-XLSX], and [MS-PPTX]. Microsoft publishes these only as text in each
  specification's appendix A, so each is read out of the specification's
  `.docx`, one paragraph a line, and kept only if its target namespace is
  listed below: a listed namespace that is missing, or found twice, is an
  error, as is a schema that does not parse.

Python's standard library only.
"""

import hashlib
import io
import sys
import urllib.request
import xml.etree.ElementTree as ET
import zipfile
from pathlib import Path

ECMA = (
    "https://ecma-international.org/wp-content/uploads/ECMA-376-4_5th_edition_december_2016.zip",
    "bd25da1109f73762356596918bf5ff8b74a1331642dba5f1c1d1dfc6bed34ecd",
)
# The transitional XML schemas, a zip inside Ecma's zip.
ECMA_SCHEMAS = "OfficeOpenXML-XMLSchema-Transitional.zip"

# Where learn.microsoft.com links each specification's dated versions.
SPECS = "https://officeprotocoldocs-f5hpbjgea6b8gneq.b02.azurefd.net/files"
MS = "http://schemas.microsoft.com/office/"

# Each specification: its pinned version, and the target namespaces taken
# from its appendix A (those the table's parts need, and what they import or
# reference).
MICROSOFT = [
    (
        "MS-DOCX",
        "260818",
        "ed5d74bb33232d083742949d8072766e99f02fc3fbc6b7dc8b73264965dcce3a",
        [
            "word/2010/wordml",
            "word/2012/wordml",
            "word/2015/wordml/symex",
            "word/2016/wordml/cid",
            "word/2018/wordml",
            "word/2018/wordml/cex",
            "word/2020/wordml/sdtdatahash",
        ],
    ),
    (
        "MS-ODRAWXML",
        "261006",
        "9ac266dbf6ebeccb23ba562614430531cba581dec118479a07ec819328f8d681",
        [
            "drawing/2008/diagram",
            "drawing/2012/chartStyle",
            "drawing/2013/main/command",
            "drawing/2014/chartex",
        ],
    ),
    (
        "MS-XLSX",
        "261118",
        "8dd600d538056ab4935c1b9b63a553667561b5d196b343ad576c10d76033d493",
        [
            "excel/2006/main",
            "spreadsheetml/2018/threadedcomments",
            "spreadsheetml/2020/threadedcomments2",
        ],
    ),
    (
        "MS-PPTX",
        "240820",
        "7530fef0768c14bee58463c896718a90f020606a0677cab221a63080e93c8170",
        [
            "powerpoint/2013/main/command",
            "powerpoint/2018/8/main",
            "powerpoint/2019/9/main/command",
            "powerpoint/2022/03/main",
            "powerpoint/2022/06/main/command",
            "powerpoint/2022/08/main",
        ],
    ),
]

W = "{http://schemas.openxmlformats.org/wordprocessingml/2006/main}"


def fetch(url, sha256):
    """The bytes at `url`, refused unless they hash to `sha256`."""
    request = urllib.request.Request(url, headers={"User-Agent": "fetch-ooxml-schemas"})
    with urllib.request.urlopen(request) as response:
        data = response.read()
    digest = hashlib.sha256(data).hexdigest()
    if digest != sha256:
        sys.exit(f"{url}: SHA-256 {digest}, expected {sha256}")
    return data


def lines(docx):
    """The text of the Word document `docx`, a paragraph a line."""
    with zipfile.ZipFile(io.BytesIO(docx)) as package:
        root = ET.fromstring(package.read("word/document.xml"))
    for paragraph in root.iter(W + "p"):
        text = []
        for node in paragraph.iter():
            if node.tag == W + "t":
                text.append(node.text or "")
            elif node.tag == W + "tab":
                text.append("\t")
            elif node.tag == W + "br":
                text.append("\n")
        yield "".join(text)


def schemas(docx):
    """Every whole schema the specification `docx` writes out, by target
    namespace: each from a line opening `<xsd:schema` to the one closing it.
    A section's fragments of a schema never open one."""
    found = {}
    current = None
    for line in lines(docx):
        if current is None and line.lstrip().startswith("<xsd:schema"):
            current = []
        if current is None:
            continue
        current.append(line)
        if "</xsd:schema>" in line:
            text = "\n".join(current) + "\n"
            current = None
            namespace = ET.fromstring(text).get("targetNamespace")
            if namespace in found and found[namespace] != text:
                sys.exit(f"two different schemas for {namespace}")
            found[namespace] = text
    return found


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    base = Path(sys.argv[1])

    ecma_dir = base / "ecma-376-4"
    ecma_dir.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(io.BytesIO(fetch(*ECMA))) as ecma:
        inner = ecma.read(ECMA_SCHEMAS)
    with zipfile.ZipFile(io.BytesIO(inner)) as transitional:
        names = [name for name in transitional.namelist() if name.endswith(".xsd")]
        for name in names:
            (ecma_dir / Path(name).name).write_bytes(transitional.read(name))
    print(f"{ecma_dir}: {len(names)} schemas from ECMA-376 Part 4")

    microsoft = base / "microsoft"
    microsoft.mkdir(parents=True, exist_ok=True)
    for spec, version, sha256, wanted in MICROSOFT:
        url = f"{SPECS}/{spec}/%5b{spec}%5d-{version}.docx"
        published = schemas(fetch(url, sha256))
        for suffix in wanted:
            namespace = MS + suffix
            if namespace not in published:
                sys.exit(f"[{spec}] {version} has no schema for {namespace}")
            name = namespace.removeprefix(MS).replace("/", "-") + ".xsd"
            (microsoft / name).write_text(published[namespace])
        print(f"{microsoft}: {len(wanted)} schemas from [{spec}] {version}")


if __name__ == "__main__":
    main()
