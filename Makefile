.DEFAULT_GOAL := help

.PHONY: help build test install clean lint

help: ## Show this help
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | \
		awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-18s\033[0m %s\n", $$1, $$2}'

build: ## Build all crates (release)
	cargo build --release

test: ## Run all tests
	cargo test --workspace

lint: ## Format and lint all code
	cargo fmt --all
	cargo clippy --workspace -- -D warnings

install: ## Install all binaries to ~/.cargo/bin
	cargo install --path crates/ledgerd
	cargo install --path crates/ledger-cli

clean: ## Remove build artifacts
	cargo clean
