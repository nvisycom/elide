// The demo's DOM surface: every element the app reads or writes, resolved once.

const $ = <T extends HTMLElement>(id: string): T => {
  const el = document.getElementById(id);
  if (!el) throw new Error(`missing #${id}`);
  return el as T;
};

export const elements = {
  input: $<HTMLTextAreaElement>("input"),
  optPatterns: $<HTMLInputElement>("opt-patterns"),
  optDictionaries: $<HTMLInputElement>("opt-dictionaries"),
  runBtn: $<HTMLButtonElement>("run"),
  status: $<HTMLSpanElement>("status"),
  result: $<HTMLDivElement>("result"),
  output: $<HTMLPreElement>("output"),
  findingsPanel: $<HTMLDivElement>("findings-panel"),
  findingsBody: $<HTMLTableSectionElement>("findings-body"),
  noFindings: $<HTMLParagraphElement>("no-findings"),
};
