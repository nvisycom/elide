# Language models

Some sensitive data depends on context: "my condition" in a medical note,
the codename that only matters inside one company. `LlmRecognizer`
(feature `llm`) asks a language model to find it, or a vision model to find
faces, signatures, and handwriting in an image.

```rust,no_run
use elide::prelude::*;
use elide::recognition::llm::LlmRecognizer;
use elide::recognition::llm::backend::RigBackend;
use elide::recognition::llm::provider::Provider;

# fn main() -> Result<()> {
let backend = RigBackend::<Text>::new(Provider::ollama("llama3.1"))?;
let llm = LlmRecognizer::with_default(backend).with_name("llama");

let analyzer = Analyzer::<Text>::new().with_recognizer(llm);
# Ok(())
# }
```

`RigBackend` (feature `llm-rig`) talks to the provider. Ollama needs no key;
OpenAI, Anthropic, and Gemini are enabled by the `openai-gpt`,
`anthropic-claude`, and `google-gemini` features of `elide-llm`, with
`Provider::openai`, `Provider::anthropic`, and `Provider::gemini`. A
`RigBackend<Image>` sends the image to a vision model instead.

## Prompts

`with_default` uses the built-in prompt for the modality. To change the
wording, implement `Prompt<M>`, or load a template with `Jinja2Prompt`
(feature `llm-jinja2`), and pass it to `LlmRecognizer::new`. The prompt
changes only the wording: the shape of the model's answer is fixed, and the
backend parses it.

## Cost

A language model is the most capable recognizer and the most expensive.
Two ways to spend less on it:

- Run it only when cheaper recognizers are unsure, with an
  `EscalatingRecognizer`; see
  [Recognizer decorators](recognizer-decorators.md#escalation).
- Measure what it costs per run with `Metered`; see
  [Usage and cost](usage.md).
