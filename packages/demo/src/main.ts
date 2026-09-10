import { elements } from "./dom";
import { Pipeline } from "./elide";
import { renderFindings } from "./render";
import "./style.css";

const el = elements;

const pipeline = new Pipeline(() => ({
  patterns: el.optPatterns.checked,
  dictionaries: el.optDictionaries.checked,
}));

// A source change means the next redaction rebuilds from the new options.
el.optPatterns.addEventListener("change", () => pipeline.invalidate());
el.optDictionaries.addEventListener("change", () => pipeline.invalidate());

async function run(): Promise<void> {
  el.runBtn.disabled = true;
  el.status.textContent = "Redacting…";
  const t0 = performance.now();
  try {
    const res = await pipeline.redact(el.input.value);
    const ms = (performance.now() - t0).toFixed(1);

    el.output.textContent = res.redacted;
    el.result.hidden = false;
    renderFindings(res.findings);
    el.status.textContent = `Done. ${res.findings.length} found in ${ms} ms.`;
  } catch (err) {
    el.status.textContent = `Error: ${err}`;
    console.error(err);
  } finally {
    el.runBtn.disabled = false;
  }
}

el.runBtn.addEventListener("click", () => void run());
el.status.textContent = "Ready.";
el.runBtn.disabled = false;
