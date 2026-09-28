VERSION := $(shell node -p "require('./extension/package.json').version")
VSIX    := extension/rustfmt-magic-$(VERSION).vsix

UNAME_S := $(shell uname -s)
UNAME_M := $(shell uname -m)
ifeq ($(UNAME_S),Darwin)
  ifeq ($(UNAME_M),arm64)
    HOST_BIN := rustfmt-magic-darwin-arm64
  else
    HOST_BIN := rustfmt-magic-darwin-x64
  endif
else
  ifeq ($(UNAME_M),aarch64)
    HOST_BIN := rustfmt-magic-linux-arm64
  else
    HOST_BIN := rustfmt-magic-linux-x64
  endif
endif

build:
	cargo build --release --manifest-path core/Cargo.toml

dev:
	watchexec -r -c -w core/src -e rs -- make build

bundle: build
	mkdir -p extension/bin
	cp core/target/release/rustfmt-magic extension/bin/$(HOST_BIN)

package: bundle
	cd extension && npx @vscode/vsce package

install: package
	code --install-extension $(VSIX)

release: package
	@test -n "$(NOTES)" || (echo 'NOTES is required: make release NOTES="..."' && exit 1)
	gh release create v$(VERSION) $(VSIX) --title "v$(VERSION)" --notes "$(NOTES)" 2>/dev/null || \
	gh release upload v$(VERSION) $(VSIX) --clobber

clean:
	cargo clean --manifest-path core/Cargo.toml
	rm -rf extension/bin extension/*.vsix

.PHONY: build dev bundle package install release clean
