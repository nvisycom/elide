//! [`ScriptPolicy`]: how the markup engine treats a `<script>` or `<style>`
//! element body.

/// How the markup engine handles a `<script>` or `<style>` element body.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScriptPolicy {
    /// Skip the element body entirely; it never enters the detection stream.
    #[default]
    Skip,
    /// Treat the element body as plain text and scan it like a text node.
    ScanText,
}
