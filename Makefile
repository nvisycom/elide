# Makefile for the Veil toolkit workspace.

ifneq (,$(wildcard ./.env))
    include .env
    export
endif

define log
printf "[%s] [MAKE] [$(MAKECMDGOALS)] $(1)\n" "$$(date '+%Y-%m-%d %H:%M:%S')"
endef

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

# `help` parses the `## …` doc comment after each target name and
# prints `target — description`. Keeping help auto-generated from
# the targets themselves means new targets don't need a manual
# entry to show up.
.PHONY: help
help:  ## Show this help.
	@awk 'BEGIN { FS = ":.*## " } /^[a-zA-Z0-9_.-]+:.*## / { printf "  %-14s  %s\n", $$1, $$2 }' $(MAKEFILE_LIST)
