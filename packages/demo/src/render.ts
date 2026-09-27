// Rendering the redaction result into the page.

import type { Entity, Location } from "@nvisy/elide";
import { elements } from "./dom";

const escapeHtml = (s: string): string =>
  s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

/** A short human-readable rendering of an entity's location. */
function where(loc: Location): string {
  switch (loc.kind) {
    case "text":
      return `${loc.start}-${loc.end}`;
    case "tabular":
      return `r${loc.row}c${loc.column}`;
    case "image":
      return `${loc.width}×${loc.height} @ ${loc.x},${loc.y}`;
    case "audio":
      return `${loc.startMs}-${loc.endMs} ms`;
    case "metadata":
      return loc.key;
  }
}

/** Fill the findings table, or show the empty state when there are none. */
export function renderFindings(entities: Entity[]): void {
  const { findingsBody, findingsPanel, noFindings } = elements;
  findingsBody.replaceChildren();
  for (const e of entities) {
    const tr = document.createElement("tr");
    tr.innerHTML =
      `<td class="font-mono">${escapeHtml(e.label)}</td>` +
      `<td>${where(e.location)}</td>` +
      `<td>${e.confidence.toFixed(2)}</td>`;
    findingsBody.appendChild(tr);
  }
  noFindings.hidden = entities.length > 0;
  findingsPanel.hidden = false;
}
