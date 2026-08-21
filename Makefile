
build-rust:
	@test -n "$(TARGET)" || (echo "TARGET is required" >&2; exit 1)
	@if [ "$(CROSS)" = "1" ]; then \
		cross build --release --target $(TARGET); \
	else \
		cargo build --release --target $(TARGET); \
	fi

check-rust:
	cargo check --all-targets --all-features

test-rust:
	cargo test --all-targets --all-features

sync-version-rust:
	@test -n "$(VERSION)" || (echo "VERSION is required" >&2; exit 1)
	sed -i.bak -E '0,/^version = "/{s/^version = ".*"/version = "$(VERSION)"/}' Cargo.toml
	@rm -f Cargo.toml.bak

publish-rust:
	@test -n "$(CARGO_TOKEN)" || (echo "CARGO_TOKEN is required" >&2; exit 1)
	cargo release publish --workspace --token $(CARGO_TOKEN) --execute --no-confirm

LANG ?= rust

release:
	@test -n "$(VERSION)" || (echo "VERSION is required" >&2; exit 1)
	NEW_VERSION=$(VERSION) ./scripts/bump_docs.sh
	@$(MAKE) sync-version-$(LANG) VERSION=$(VERSION)
	./scripts/release.sh "$(VERSION)"
