SHELL := /bin/bash
CANN_ROOT ?= /usr/local/Ascend/ascend-toolkit/latest
ASCEND_ARCH ?= dav-2201

.PHONY: test build package release clean

test:
	cargo test --no-default-features --features stub,node

build:
	@test -f "$(CANN_ROOT)/set_env.sh" || (echo "CANN not found: $(CANN_ROOT)/set_env.sh" >&2; exit 1)
	@source "$(CANN_ROOT)/set_env.sh"; \
	ASC_MODULES="$${ASC_MODULES:-$$(find "$(CANN_ROOT)" -type d -path '*/ascendc_kernel_cmake/asc_modules' -print -quit)}"; \
	test -n "$$ASC_MODULES" || (echo "ASC_MODULES not found" >&2; exit 1); \
	ASC_MODULES="$$ASC_MODULES" ASCEND_ARCH="$(ASCEND_ARCH)" cargo build --release --no-default-features --features cann,node

package:
	@set -e; BIN=target/release/ascend-nockchain-miner; test -x $$BIN; \
	SO=$$(find target/release/build -name libascend_miner.so -print -quit); test -n "$$SO"; \
	VER=$$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -1); \
	SHA=$${GITHUB_SHA:-local}; SHA=$${SHA:0:7}; NAME=ascend-nockchain-miner-$$VER-$$SHA-linux-arm64-910b; \
	rm -rf dist; mkdir -p dist/$$NAME; cp $$BIN $$SO README.md dist/$$NAME/; \
	tar -C dist -czf dist/$$NAME.tar.gz $$NAME; sha256sum dist/$$NAME.tar.gz > dist/$$NAME.tar.gz.sha256; rm -rf dist/$$NAME

release: build package

clean:
	rm -rf target dist
