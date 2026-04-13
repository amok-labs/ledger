.DEFAULT_GOAL := help

.PHONY: help build test install clean check fmt clippy
.PHONY: build-ledgerd build-cli build-tui
.PHONY: test-ledgerd test-cli test-client
.PHONY: install-ledgerd install-cli install-tui

help: ## Show this help
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | \
		awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-18s\033[0m %s\n", $$1, $$2}'

# --- Workspace-wide targets ---

build: ## Build all crates (release)
	cargo build --release

test: ## Run all tests
	cargo test --workspace

install: install-ledgerd install-cli install-tui ## Install all binaries to ~/.cargo/bin

clean: ## Remove build artifacts
	cargo clean

check: ## Type-check workspace
	cargo check --workspace

fmt: ## Format all code
	cargo fmt --all

clippy: ## Lint all code
	cargo clippy --workspace -- -D warnings

# --- Per-crate targets ---

build-ledgerd: ## Build daemon (release)
	cargo build --release -p ledgerd

build-cli: ## Build CLI (release)
	cargo build --release -p ledger-cli

build-tui: ## Build TUI (release)
	cargo build --release -p ledger-tui

test-ledgerd: ## Test daemon
	cargo test -p ledgerd

test-cli: ## Test CLI
	cargo test -p ledger-cli

test-client: ## Test client library
	cargo test -p ledger-client

install-ledgerd: ## Install daemon to ~/.cargo/bin
	cargo install --path crates/ledgerd

install-cli: ## Install CLI to ~/.cargo/bin
	cargo install --path crates/ledger-cli

install-tui: ## Install TUI to ~/.cargo/bin
	cargo install --path crates/ledger-tui
