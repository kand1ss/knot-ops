KNOTD_DIR := components/knotd
BIN_DIR   := bin

HOST_TARGET := $(shell rustc -vV 2>/dev/null | sed -n 's/^host: //p')
HOST_GOOS   := $(shell go env GOOS 2>/dev/null)
HOST_GOARCH := $(shell go env GOARCH 2>/dev/null)

TARGET ?= $(HOST_TARGET)
GOOS   ?= $(HOST_GOOS)
GOARCH ?= $(HOST_GOARCH)

.PHONY: all build build-rust build-go check check-rust check-go \
        test test-rust test-go prepare-release release \
        sync-version-rust publish-rust

all: check test

build: build-rust build-go

check: check-rust check-go

test: test-rust test-go

# Go targets
build-go:
	@test -n "$(GOOS)" || (echo "GOOS is required" >&2; exit 1)
	@test -n "$(GOARCH)" || (echo "GOARCH is required" >&2; exit 1)
	@set -e; \
	EXT=""; [ "$(GOOS)" = "windows" ] && EXT=".exe"; \
	mkdir -p $(BIN_DIR); \
	echo "Building knotd for $(GOOS)/$(GOARCH)..."; \
	CGO_ENABLED=0 GOOS=$(GOOS) GOARCH=$(GOARCH) go build \
		-C $(KNOTD_DIR) \
		-trimpath \
		-ldflags="-s -w $(if $(VERSION),-X main.version=$(VERSION))" \
		-o "$(CURDIR)/$(BIN_DIR)/knotd$$EXT" ./cmd/knotd

check-go:
	cd $(KNOTD_DIR) && go vet ./...

test-go:
	cd $(KNOTD_DIR) && go test -v -race ./...

# Rust targets
build-rust:
	@test -n "$(TARGET)" || (echo "TARGET is required" >&2; exit 1)
	@if [ "$(CROSS)" = "1" ]; then \
		cross build --release --target $(TARGET); \
	else \
		cargo build --release --target $(TARGET); \
	fi

check-rust:
	cargo check --workspace --all-targets --all-features

test-rust:
	cargo test --workspace --all-targets --all-features

sync-version-rust:
	@test -n "$(VERSION)" || (echo "VERSION is required" >&2; exit 1)
	@command -v cargo-release >/dev/null 2>&1 || \
		(echo "Error: cargo-release not found (cargo install cargo-release)" >&2; exit 1)
	cargo release version $(VERSION) --workspace --execute --no-confirm

publish-rust:
	@test -n "$(CARGO_TOKEN)" || (echo "CARGO_TOKEN is required" >&2; exit 1)
	cargo release publish --workspace --token $(CARGO_TOKEN) --execute --no-confirm

# Release & automation
prepare-release:
	@test -n "$(VERSION)" || (echo "VERSION is required" >&2; exit 1)
	NEW_VERSION=$(VERSION) ./scripts/bump_docs.sh
	@$(MAKE) sync-version-rust VERSION=$(VERSION)

release:
	@test -n "$(VERSION)" || (echo "VERSION is required" >&2; exit 1)
	@$(MAKE) prepare-release VERSION=$(VERSION)
	./scripts/release.sh "$(VERSION)"