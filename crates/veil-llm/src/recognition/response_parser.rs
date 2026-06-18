//! Forgiving JSON parser for LLM responses.
//!
//! Strips markdown fences when present. Returns `T::default()` for
//! empty / `"none"` / `"no entities"` responses.

use serde::de::DeserializeOwned;

/// Parse `text` as JSON into `T`, with markdown-fence + sentinel
/// fallback handling.
pub(super) fn parse_json<T: DeserializeOwned + Default>(
    text: &str,
) -> Result<T, serde_json::Error> {
    let trimmed = text.trim();

    if trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case("none")
        || trimmed.eq_ignore_ascii_case("no entities")
    {
        return Ok(T::default());
    }

    let json_str = extract_fenced_json(trimmed).unwrap_or(trimmed);
    serde_json::from_str::<T>(json_str)
}

/// Extract JSON content from markdown fences (```` ```json ... ``` ````).
fn extract_fenced_json(text: &str) -> Option<&str> {
    let start_marker = if let Some(pos) = text.find("```json") {
        pos + "```json".len()
    } else if let Some(pos) = text.find("```") {
        pos + "```".len()
    } else {
        return None;
    };

    let rest = &text[start_marker..];
    let rest = rest.strip_prefix('\n').unwrap_or(rest);
    let end = rest.find("```")?;
    let content = rest[..end].trim();

    if content.is_empty() {
        None
    } else {
        Some(content)
    }
}
