# Makefile for the Elide toolkit workspace.

ifneq (,$(wildcard ./.env))
    include .env
    export
endif

define log
printf "[%s] [MAKE] [$(MAKECMDGOALS)] $(1)\n" "$$(date '+%Y-%m-%d %H:%M:%S')"
endef

.PHONY: install-deps
install-deps: ## Installs system build dependencies (C toolchain for the mp3 codec).
	@$(call log,Installing build-essential autoconf automake...)
	@sudo apt-get update && sudo apt-get install -y build-essential autoconf automake
	@$(call log,Build dependencies installed.)

.PHONY: install-pdfium
install-pdfium: ## Installs the shared libraries
	@$(call log,Installing PDFium shared library...)
	@chmod +x scripts/*.sh
	@./scripts/install-pdfium.sh
	@$(call log,PDFium installed.)

.PHONY: install-tools
install-tools: ## Installs CLI tools required for development.
	@$(call log,Checking cargo-watch...)
	@if ! command -v cargo-watch >/dev/null 2>&1; then \
		$(call log,Installing cargo-watch...); \
		cargo install cargo-watch --locked; \
		$(call log,cargo-watch installed.); \
	else \
		$(call log,cargo-watch already installed.); \
	fi

.PHONY: wasm-pkg
wasm-pkg: ## Builds elide-wasm and generates its JS/TS bindings into www/pkg.
	@$(call log,Adding wasm32 target...)
	@rustup target add wasm32-unknown-unknown
	@chmod +x scripts/install-wasm-bindgen.sh
	@./scripts/install-wasm-bindgen.sh
	@$(call log,Building elide-wasm (release)...)
	@cargo build -p elide-wasm --target wasm32-unknown-unknown --release
	@$(call log,Generating JS/TS bindings...)
	@wasm-bindgen target/wasm32-unknown-unknown/release/elide_wasm.wasm \
		--out-dir crates/elide-wasm/www/pkg --target web
	@$(call log,Bindings written to crates/elide-wasm/www/pkg.)

.PHONY: wasm-demo
wasm-demo: wasm-pkg ## Builds the full browser demo (elide-wasm + Vite app).
	@$(call log,Building demo (Vite)...)
	@cd crates/elide-wasm/www && npm ci && npm run build
	@$(call log,Demo built to crates/elide-wasm/www/dist.)
	@$(call log,Preview it with: cd crates/elide-wasm/www && npm run preview)

.PHONY: wasm-dev
wasm-dev: wasm-pkg ## Runs the demo dev server with hot reload.
	@cd crates/elide-wasm/www && npm install && npm run dev

.PHONY: lint
lint: ## Runs clippy and format check.
	@$(call log,Running format check...)
	@cargo fmt --all -- --check
	@$(call log,Running clippy...)
	@cargo clippy --workspace -- -D warnings
	@$(call log,Lint passed.)

.PHONY: ci
ci: lint ## Runs all CI checks locally.
	@cargo check --workspace
	@cargo test --workspace
	@cargo build --workspace --release
	@$(call log,All CI checks passed!)

.PHONY: clean-testdata
clean-testdata: ## Removes generated e2e artifacts (testdata/audits, testdata/results).
	@$(call log,Cleaning generated e2e test artifacts...)
	@find . -type d -path '*/testdata/*' \( -name audits -o -name results \) | while read -r dir; do \
		find "$$dir" -type f ! -name .gitkeep -delete; \
	done
	@$(call log,Test artifacts cleaned.)

# `help` parses the `## …` doc comment after each target name and
# prints `target — description`. Keeping help auto-generated from
# the targets themselves means new targets don't need a manual
# entry to show up.
.PHONY: help
help:  ## Show this help.
	@awk 'BEGIN { FS = ":.*## " } /^[a-zA-Z0-9_.-]+:.*## / { printf "  %-14s  %s\n", $$1, $$2 }' $(MAKEFILE_LIST)
