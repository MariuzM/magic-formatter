VERSION := $(shell node -p "require('./extension/package.json').version")
VSIX    := extension/magic-formatter-$(VERSION).vsix

UNAME_S := $(shell uname -s)
UNAME_M := $(shell uname -m)
ifeq ($(UNAME_S),Darwin)
  ifeq ($(UNAME_M),arm64)
    HOST_BIN := magic-formatter-darwin-arm64
  else
    HOST_BIN := magic-formatter-darwin-x64
  endif
else
  ifeq ($(UNAME_M),aarch64)
    HOST_BIN := magic-formatter-linux-arm64
  else
    HOST_BIN := magic-formatter-linux-x64
  endif
endif

build:
	cargo build --release

dev:
	watchexec -r -c -w crates -e rs -- make bundle

bundle: build
	mkdir -p extension/bin
	cp target/release/magic-formatter extension/bin/$(HOST_BIN)

package: bundle
	cd extension && npm ci && npx @vscode/vsce package --no-dependencies

install: package
	code --install-extension $(VSIX)

release: package
	@test -n "$(NOTES)" || (echo 'NOTES is required: make release NOTES="..."' && exit 1)
	gh release create v$(VERSION) $(VSIX) --title "v$(VERSION)" --notes "$(NOTES)" 2>/dev/null || \
	gh release upload v$(VERSION) $(VSIX) --clobber

clean:
	cargo clean
	rm -rf extension/bin extension/dist extension/*.vsix

.PHONY: build dev bundle package install release clean
