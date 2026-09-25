//! Standalone value types the plain-text codecs are configured with:
//! [`ScriptPolicy`] (whether an HTML `<script>` / `<style>` body is scanned).

#[cfg(feature = "html")]
mod script_policy;

#[cfg(feature = "html")]
pub use self::script_policy::ScriptPolicy;
