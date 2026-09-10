// Rendering the redaction result into the page.

import { elements } from "./dom";
import type { Finding } from "./elide";

const escapeHtml = (s: string): string =>
  s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

/** Fill the findings table, or show the empty state when there are none. */
export function renderFindings(findings: Finding[]): void {
  const { findingsBody, findingsPanel, noFindings } = elements;
  findingsBody.replaceChildren();
  for (const f of findings) {
    const tr = document.createElement("tr");
    tr.innerHTML =
      `<td class="font-mono">${escapeHtml(f.label)}</td>` +
      `<td>${f.start}-${f.end}</td>` +
      `<td>${f.confidence.toFixed(2)}</td>`;
    findingsBody.appendChild(tr);
  }
  noFindings.hidden = findings.length > 0;
  findingsPanel.hidden = false;
}
