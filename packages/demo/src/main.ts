import {
  createAnalyzer,
  createAnonymizer,
  createPatternRecognizer,
  redact,
  type AnalyzerHandle,
  type AnonymizerHandle,
  type Finding,
} from "./elide";
import "./style.css";

const $ = <T extends HTMLElement>(id: string): T => {
  const el = document.getElementById(id);
  if (!el) throw new Error(`missing #${id}`);
  return el as T;
};

const input = $<HTMLTextAreaElement>("input");
const optPatterns = $<HTMLInputElement>("opt-patterns");
const optDictionaries = $<HTMLInputElement>("opt-dictionaries");
const runBtn = $<HTMLButtonElement>("run");
const statusEl = $<HTMLSpanElement>("status");
const result = $<HTMLDivElement>("result");
const output = $<HTMLPreElement>("output");
const findingsPanel = $<HTMLDivElement>("findings-panel");
const findingsBody = $<HTMLTableSectionElement>("findings-body");
const noFindings = $<HTMLParagraphElement>("no-findings");

// The analyzer only depends on the two source checkboxes, so build the pipeline
// once and reuse it; rebuild lazily when a checkbox changes.
let pipeline: { analyzer: AnalyzerHandle; anonymizer: AnonymizerHandle } | null =
  null;

function invalidatePipeline(): void {
  pipeline?.analyzer.free();
  pipeline?.anonymizer.free();
  pipeline = null;
}
optPatterns.addEventListener("change", invalidatePipeline);
optDictionaries.addEventListener("change", invalidatePipeline);

const escapeHtml = (s: string): string =>
  s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

function renderFindings(findings: Finding[]): void {
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

async function run(): Promise<void> {
  runBtn.disabled = true;
  statusEl.textContent = "Redacting…";
  const t0 = performance.now();
  try {
    if (!pipeline) {
      const recognizer = createPatternRecognizer({
        builtinPatterns: optPatterns.checked,
        builtinDictionaries: optDictionaries.checked,
      });
      pipeline = {
        analyzer: createAnalyzer([recognizer]),
        anonymizer: createAnonymizer(),
      };
    }
    const res = await redact(pipeline.analyzer, pipeline.anonymizer, input.value);
    const ms = (performance.now() - t0).toFixed(1);

    output.textContent = res.redacted;
    result.hidden = false;
    renderFindings(res.findings);
    statusEl.textContent = `Done. ${res.findings.length} found in ${ms} ms.`;
  } catch (err) {
    statusEl.textContent = `Error: ${err}`;
    console.error(err);
  } finally {
    runBtn.disabled = false;
  }
}

function main(): void {
  runBtn.addEventListener("click", () => void run());
  statusEl.textContent = "Ready.";
  runBtn.disabled = false;
}

main();
