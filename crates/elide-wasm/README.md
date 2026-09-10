# elide-wasm

[![Build](https://img.shields.io/github/actions/workflow/status/nvisycom/elide/build.yml?branch=main&label=build%20%26%20test&style=flat-square)](https://github.com/nvisycom/elide/actions/workflows/build.yml)

WebAssembly bindings that run the Elide detect-and-redact pipeline in a browser.

## Overview

This crate is a thin WebAssembly boundary over the Elide facade. It exposes a
single asynchronous entry point that takes a piece of text, runs the built-in
pattern and dictionary recognizers over it, applies a per-label redaction
policy, and returns the redacted text alongside the list of entities that were
found. It carries no detection logic of its own; the whole pipeline is the same
one the native toolkit runs.

The pipeline is asynchronous, and in the browser its work is driven by the
page's own event loop rather than by a Rust async runtime, so nothing here
spawns threads, opens sockets, or touches a filesystem. Only the parts of the
toolkit that compile to WebAssembly are pulled in: pattern detection, the
redaction operators, the pseudonymizer, and the plain-text codec. The
model-backed, native-codec, and network features (audio, PDF rendering, hosted
language models) are deliberately left out of the browser build.

The `www` directory holds a small TypeScript and Vite application that imports
the compiled module and redacts text entirely on the client, with no data
leaving the browser. The Rust result types are surfaced to TypeScript through
generated type definitions, so the browser code is fully typed. Run
`make wasm-dev` for a hot-reloading dev server or `make wasm-demo` to produce the
static build; the same static build is published to GitHub Pages on each change
to `main`.
