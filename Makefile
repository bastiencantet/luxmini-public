.PHONY: help build run setup app dmg release lint test clean
.DEFAULT_GOAL := help

HELPER = target/release/led-helper
APP    = target/release/mac-led-tray

# Dev run needs Sparkle.framework discoverable at runtime.
export DYLD_FRAMEWORK_PATH := $(CURDIR)/vendor

help: ## Show this help
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) \
		| awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-9s\033[0m %s\n", $$1, $$2}'

build: ## Build both binaries (release)
	cargo build --release --bin led-helper
	cargo build --release --bin mac-led-tray

run: build ## Build and run the tray app
	$(APP)

# One-time: make the helper setuid root so SMC writes work without sudo.
# Only needed during dev — the bundled app auto-elevates on first launch.
setup: build ## Install the dev helper as setuid root (admin prompt)
	@echo "Installing led-helper with admin privileges..."
	osascript -e "do shell script \"chown root '$(CURDIR)/$(HELPER)' && chmod u+s '$(CURDIR)/$(HELPER)'\" with administrator privileges"
	@echo "Done. Run: make run"

app: ## Build the LuxMini.app bundle under dist/
	./scripts/bundle.sh

# Override version via `make dmg VERSION=0.1.2`; otherwise it reads Cargo.toml.
dmg: ## Build the app and package it as dist/LuxMini-<version>.dmg
	VERSION=$(VERSION) ./scripts/dmg.sh

# Usage: make release VERSION=0.2.3 NOTES="Fix slider padding"
release: ## Cut a signed release (dmg + appcast + upload)
	./scripts/release.sh "$(VERSION)" "$(NOTES)"

lint: ## Run the full local quality gate (matches CI)
	cargo fmt --all --check
	cargo clippy --all-targets -- -D warnings
	@command -v shellcheck >/dev/null 2>&1 && shellcheck scripts/*.sh \
		|| echo "shellcheck not installed — skipping shell lint"

test: ## Run the test suite
	cargo test --all-targets

clean: ## Remove build artifacts and dist/
	cargo clean
	rm -rf dist
