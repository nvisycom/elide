//! Emit every built-in label id as a JSON array on stdout.
//!
//! The single source of truth for the generated `Label` constants in the
//! `@nvisy/elide` package. `scripts/gen-labels.mjs` runs this (via `cargo run
//! --example labels`) and writes `packages/wasm/label.{js,d.ts}`.

fn main() {
    let ids = elide_wasm::builtin_label_ids();
    let json: Vec<String> = ids.iter().map(|id| format!("{id:?}")).collect();
    println!("[{}]", json.join(","));
}
