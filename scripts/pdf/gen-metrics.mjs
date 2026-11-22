// Generate `crates/elide-pdf/src/pdf/reader/font/standard/metrics.rs`: the
// metrics of the 14 standard fonts (9.6.2.2), from Adobe's Core14 AFM files.
//
// Fetches Adobe's `Core14_AFMs.zip`, checks it against a pinned SHA-256, and
// writes, for each face, what elide-pdf reads: each glyph's code under the
// face's built-in encoding and its width, and the
// `CapHeight`, `Ascender`, `Descender`, `FontBBox`, `ItalicAngle` and `StdVW`
// its font descriptor is given. Kerning, ligatures and glyph boxes are left
// out. The faces are written in the order of `Face` in `standard/mod.rs`, as
// input to its `face!` macro.
//
// Adobe's licence, the archive's `MustRead.html`, allows modified copies that
// keep the copyright notices, note the modification, and travel with it: the
// generated file does the first two, and the licence is written unmodified
// beside it.
//
// Usage: node scripts/pdf/gen-metrics.mjs

import { createHash } from "node:crypto";
import { writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { inflateRawSync } from "node:zlib";

const URL = "https://download.macromedia.com/pub/developer/opentype/tech-notes/Core14_AFMs.zip";
const SHA256 = "8c892c3c49553cfd2d2a27c4495b4bb12e2875115be7fd127ed3876df19d8654";
const FACES = [
  "Courier",
  "Courier-Bold",
  "Courier-Oblique",
  "Courier-BoldOblique",
  "Helvetica",
  "Helvetica-Bold",
  "Helvetica-Oblique",
  "Helvetica-BoldOblique",
  "Times-Roman",
  "Times-Bold",
  "Times-Italic",
  "Times-BoldItalic",
  "Symbol",
  "ZapfDingbats",
];

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const dir = join(root, "crates/elide-pdf/src/pdf/reader/font/standard");

// The members of the zip `data`, by name, read from its central directory.
function unzip(data) {
  const end = data.lastIndexOf(Buffer.from([0x50, 0x4b, 0x05, 0x06]));
  if (end < 0) throw new Error("no end of central directory");
  const count = data.readUInt16LE(end + 10);
  let at = data.readUInt32LE(end + 16);
  const members = new Map();
  for (let i = 0; i < count; i++) {
    if (data.readUInt32LE(at) !== 0x02014b50) throw new Error("bad central directory");
    const method = data.readUInt16LE(at + 10);
    const size = data.readUInt32LE(at + 20);
    const nameLength = data.readUInt16LE(at + 28);
    const extraLength = data.readUInt16LE(at + 30);
    const commentLength = data.readUInt16LE(at + 32);
    const local = data.readUInt32LE(at + 42);
    const name = data.toString("latin1", at + 46, at + 46 + nameLength);
    const start = local + 30 + data.readUInt16LE(local + 26) + data.readUInt16LE(local + 28);
    const body = data.subarray(start, start + size);
    if (method !== 0 && method !== 8) throw new Error(`${name}: compression ${method}`);
    members.set(name.split("/").pop(), method === 8 ? inflateRawSync(body) : body);
    at += 46 + nameLength + extraLength + commentLength;
  }
  return members;
}

// The number `text` spells, or an error naming `line`.
function finite(text, line) {
  const value = Number(text);
  if (text === "" || !Number.isFinite(value)) throw new Error(`not a number: ${line}`);
  return value;
}

// What elide-pdf reads from the AFM `text`.
function parse(text) {
  const face = { notices: [], glyphs: new Map() };
  for (const line of text.split(/\r?\n/)) {
    if (line.startsWith("C ")) {
      const fields = line.slice(2).split(";").map((field) => field.trim());
      const code = Number(fields[0]);
      const width = fields.find((field) => field.startsWith("WX "))?.slice(3).trim();
      const name = fields.find((field) => field.startsWith("N "))?.slice(2).trim();
      if (!Number.isInteger(code) || width === undefined || name === undefined) {
        throw new Error(`an unreadable glyph: ${line}`);
      }
      if (code < -1 || code > 255) throw new Error(`a code past one byte: ${line}`);
      if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(name)) throw new Error(`a name the macro can't take: ${line}`);
      face.glyphs.set(name, { code, width: finite(width, line) });
      continue;
    }
    const [key] = line.split(/\s+/);
    const value = line.slice(key.length).trim();
    switch (key) {
      case "FontName":
        face.name = value;
        break;
      case "Notice":
        face.notices.push(value);
        break;
      case "CapHeight":
      case "Ascender":
      case "Descender":
      case "ItalicAngle":
      case "StdVW":
        face[key] = finite(value, line);
        break;
      case "FontBBox":
        face.FontBBox = value.split(/\s+/).map((corner) => finite(corner, line));
        if (face.FontBBox.length !== 4) throw new Error(`not four corners: ${line}`);
        break;
    }
  }
  return face;
}

const constant = (name) => name.toUpperCase().replaceAll("-", "_");
// Glyphs to a line, and the width each takes, so they line up in columns.
const PER_LINE = 4;
const COLUMN = 24;

const response = await fetch(URL);
if (!response.ok) throw new Error(`Core14_AFMs.zip: HTTP ${response.status}`);
const archive = Buffer.from(await response.arrayBuffer());
const digest = createHash("sha256").update(archive).digest("hex");
if (digest !== SHA256) throw new Error(`Core14_AFMs.zip: SHA-256 ${digest}, pinned ${SHA256}`);
const members = unzip(archive);

const faces = FACES.map((name) => {
  const member = members.get(`${name}.afm`);
  if (!member) throw new Error(`${name}.afm is not in the archive`);
  const face = parse(member.toString("latin1"));
  if (face.name !== name) throw new Error(`${name}.afm names ${face.name}`);
  return face;
});

const notices = [...new Set(faces.flatMap((face) => face.notices))];
let rust = `//! The metrics of the 14 standard fonts, generated by
//! \`scripts/pdf/gen-metrics.mjs\` from Adobe's Core14 AFM files; do not edit.
//!
//! Modified from the AFM files: only each glyph's name, built-in code (-1 for
//! none) and width, and the face's cap height, ascender, descender, bounding
//! box, italic angle and dominant vertical stem, are kept, written as input to
//! [\`face!\`]. The AFM files are
//! distributed with Adobe's licence, \`MustRead.html\` beside this file.
//! Their notices:
//!
${notices.map((notice) => `//! - ${notice}`).join("\n")}

#![cfg_attr(rustfmt, rustfmt::skip)]

use super::Metrics;

/// Each face's metrics, in the order of [\`Face\`](super::Face).
pub(super) static FACES: [Metrics; ${faces.length}] = [
${faces.map((face) => `    ${constant(face.name)},`).join("\n")}
];
`;
for (const face of faces) {
  const glyphs = [...face.glyphs]
    .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
    .map(([name, { code, width }]) => `${name} ${code} ${width},`);
  const lines = [];
  for (let i = 0; i < glyphs.length; i += PER_LINE) {
    const line = glyphs.slice(i, i + PER_LINE).map((glyph) => glyph.padEnd(COLUMN));
    lines.push(`        ${line.join("").trimEnd()}`);
  }
  const header = [
    ["cap_height", face.CapHeight],
    ["ascender", face.Ascender],
    ["descender", face.Descender],
  ].filter(([, value]) => value !== undefined);
  rust += `
const ${constant(face.name)}: Metrics = face! {
    "${face.name}",
${header.map(([key, value]) => `    ${key}: ${value},`).join("\n")}${header.length ? "\n" : ""}    bbox: [${face.FontBBox.join(", ")}],
    italic_angle: ${face.ItalicAngle ?? 0},
    stem_v: ${face.StdVW ?? 0},
    glyphs: [
${lines.join("\n")}
    ]
};
`;
}
const licence = members.get("MustRead.html");
if (!licence) throw new Error("MustRead.html is not in the archive");
writeFileSync(join(dir, "metrics.rs"), rust);
writeFileSync(join(dir, "MustRead.html"), licence);
console.log(`wrote ${faces.length} faces and Adobe's licence to ${dir}`);
