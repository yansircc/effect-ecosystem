PREFIX ?= $(HOME)/.local
BIN ?= $(PREFIX)/bin/effect-scan
BUILD_DIR ?= dist-dev
INSTALL_ROOT ?= dist-installed

.PHONY: build install package skill verify verify-acceptance verify-performance verify-performance-realworld

build:
	rm -rf "$(BUILD_DIR)"
	node scripts/prepare-effect-tsgo.mjs
	./node_modules/.bin/tsc -p tsconfig.json --outDir "$(BUILD_DIR)"
	node "$(BUILD_DIR)/scripts/write-build-info.js" "$(BUILD_DIR)"
	mkdir -p "$(BUILD_DIR)/validator/fixtures" "$(BUILD_DIR)/references" "$(BUILD_DIR)/contracts/evidence"
	cp validator/manifest.schema.json "$(BUILD_DIR)/validator/"
	cp validator/fixtures/fixtures.config.json "$(BUILD_DIR)/validator/fixtures/"
	cp -R references/. "$(BUILD_DIR)/references/"
	cp contracts/*.json "$(BUILD_DIR)/contracts/"
	cp contracts/evidence/*.json "$(BUILD_DIR)/contracts/evidence/"
	chmod +x "$(BUILD_DIR)/validator/scan.js"

install:
	@set -eu; \
	mkdir -p "$(INSTALL_ROOT)" "$(dir $(BIN))"; \
	tmp="$(INSTALL_ROOT)/.tmp-install-$$$$"; \
	rm -rf "$$tmp"; \
	$(MAKE) build BUILD_DIR="$$tmp"; \
	build_id=$$(node -e 'const fs=require("fs"); console.log(JSON.parse(fs.readFileSync(process.argv[1], "utf8")).buildId)' "$$tmp/build-info.json"); \
	target="$(INSTALL_ROOT)/$$build_id"; \
	if [ -e "$$target" ]; then rm -rf "$$tmp"; else mv "$$tmp" "$$target"; fi; \
	ln -sfn "$(CURDIR)/$$target/validator/scan.js" "$(BIN)"

package:
	$(MAKE) build BUILD_DIR=dist/build
	mkdir -p dist/empty dist/skill
	rsync -a --delete dist/empty/ dist/skill/
	mkdir -p dist/skill/runtime/validator dist/skill/runtime/fixtures dist/skill/runtime/node_modules
	bun build validator/scan.ts --target=node --external jsonc-parser --outfile dist/skill/runtime/validator/scan.js
	rsync -a --delete dist/build/contracts/ dist/skill/contracts/
	rsync -a --delete dist/build/references/ dist/skill/references/
	rsync -a --delete node_modules/jsonc-parser/ dist/skill/runtime/node_modules/jsonc-parser/
	@set -eu; \
	archive_dir=$$(mktemp -d "dist/.typescript-platforms.XXXXXX"); \
	trap 'rm -rf "$$archive_dir"' EXIT; \
	version=$$(node -p 'require("./package.json").dependencies.typescript'); \
	for platform in darwin-arm64 linux-x64 linux-arm64; do \
		package="@typescript/typescript-$$platform"; \
		archive=$$(npm pack "$$package@$$version" --pack-destination "$$archive_dir" --silent | tail -1); \
		target="dist/skill/runtime/node_modules/@typescript/typescript-$$platform"; \
		mkdir -p "$$target"; \
		tar -xzf "$$archive_dir/$$archive" -C "$$target" --strip-components=1; \
	done
	cp packaging/typescript-runtime.package.json dist/skill/runtime/package.json
	cp dist/build/build-info.json dist/skill/
	cp validator/manifest.schema.json dist/skill/runtime/
	cp validator/fixtures/fixtures.config.json dist/skill/runtime/fixtures/
	rsync -a --delete SKILL.md agents dist/skill/

skill: package

verify:
	@set -eu; \
	snapshot() { target=$$(readlink "$(BIN)" 2>/dev/null || true); if [ -n "$$target" ] && [ -f "$$target" ]; then hash=$$(shasum -a 256 "$$target" | awk '{print $$1}'); else hash="missing"; fi; printf "%s|%s" "$$target" "$$hash"; }; \
	before=$$(snapshot); \
	status=0; \
	{ \
	$(MAKE) build BUILD_DIR="$(BUILD_DIR)" && \
	node "$(BUILD_DIR)/scripts/verify-build-provenance.js" && \
	node "$(BUILD_DIR)/scripts/check-registry.js" && \
	node "$(BUILD_DIR)/scripts/generate-derived-docs.js" --check && \
	node "$(BUILD_DIR)/scripts/check-skill-thin.js" && \
	node "$(BUILD_DIR)/scripts/check-profile-routing.js" && \
	node "$(BUILD_DIR)/scripts/check-evidence-contracts.js" && \
	node "$(BUILD_DIR)/scripts/check-signal-contracts.js" && \
	node "$(BUILD_DIR)/scripts/check-effect-capabilities.js" && \
	node "$(BUILD_DIR)/scripts/check-reference-routing.js" && \
	node "$(BUILD_DIR)/scripts/typecheck-snippets.js" && \
	node "$(BUILD_DIR)/scripts/self-test.js" --exact && \
	node "$(BUILD_DIR)/scripts/self-test.js" --contract && \
	node "$(BUILD_DIR)/validator/scan.js" . --strict --output gate-json >/tmp/effect-skill-verify-profile.json && \
	node "$(BUILD_DIR)/scripts/verify-acceptance.js" && \
	node "$(BUILD_DIR)/scripts/benchmark-scan.js"; \
	} || status=$$?; \
	after=$$(snapshot); \
	if [ "$$before" != "$$after" ]; then echo "installed scanner changed during make verify: $$before -> $$after" >&2; exit 1; fi; \
	exit "$$status"

verify-acceptance: build
	node "$(BUILD_DIR)/scripts/verify-acceptance.js"

verify-performance: build
	node "$(BUILD_DIR)/scripts/benchmark-scan.js"

verify-performance-realworld: build
	node "$(BUILD_DIR)/scripts/benchmark-scan.js" --github
