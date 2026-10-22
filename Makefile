# One build output: $(DIST)/ (default dist/).
#   $(DIST)/platforms/<platform>/effect-scan   the native binary (+ build-info.json it embeds)
#   $(DIST)/tools/                             TypeScript development tools (self-test, checks, oracle); never shipped
#   $(DIST)/skill/                             the assembled skill        (make skill)
#   $(DIST)/npm/                               the npm release set        (make npm-package)
# One install path: $(BIN) -> $(INSTALL_ROOT)/<buildId>/effect-scan   (make install)
PREFIX ?= $(HOME)/.local
BIN ?= $(PREFIX)/bin/effect-scan
INSTALL_ROOT ?= $(PREFIX)/lib/effect-scan
DIST ?= dist
TOOLS := $(DIST)/tools
HOST := $(shell node scripts/release/platforms.mjs host)
BINARY := $(DIST)/platforms/$(HOST)/effect-scan
BUILD_INFO := $(TOOLS)/build-info.json
# Platforms without a build become marked placeholders (structure checks only). CI passes PACK_FLAGS= to require all.
PACK_FLAGS ?= --placeholder-missing

.PHONY: tools build diff-fixtures install skill npm-package package verify verify-offline verify-online verify-acceptance verify-performance verify-performance-realworld

tools:
	rm -rf "$(TOOLS)"
	node scripts/prepare-effect-tsgo.mjs
	./node_modules/.bin/tsc -p tsconfig.json --outDir "$(TOOLS)"
	node "$(TOOLS)/scripts/write-build-info.js" "$(TOOLS)"
	mkdir -p "$(TOOLS)/validator/fixtures" "$(TOOLS)/references" "$(TOOLS)/contracts/evidence"
	cp validator/manifest.schema.json "$(TOOLS)/validator/"
	cp validator/fixtures/fixtures.config.json "$(TOOLS)/validator/fixtures/"
	cp -R references/. "$(TOOLS)/references/"
	cp contracts/*.json "$(TOOLS)/contracts/"
	cp contracts/evidence/*.json "$(TOOLS)/contracts/evidence/"
	chmod +x "$(TOOLS)/validator/scan.js"

# The binary embeds the build provenance written by `tools` (scanner.buildId, complianceHash).
build: tools
	EFFECT_SCAN_BUILD_INFO="$(abspath $(BUILD_INFO))" cargo build --release --locked -p effect-scan
	mkdir -p "$(dir $(BINARY))"
	cp target/release/effect-scan "$(BINARY)"
	cp "$(BUILD_INFO)" "$(dir $(BINARY))build-info.json"
	node scripts/release/check-binary.mjs "$(BINARY)" "$(BUILD_INFO)"

# The differential harness needs the binary built with the oracle's build-info, which `build` guarantees.
diff-fixtures: build
	node "$(TOOLS)/scripts/diff-fixtures.js" "$(BINARY)"

install: build
	@set -eu; \
	build_id=$$(node -p 'JSON.parse(require("fs").readFileSync(process.argv[1], "utf8")).buildId' "$(BUILD_INFO)"); \
	target="$(abspath $(INSTALL_ROOT))/$$build_id/effect-scan"; \
	mkdir -p "$$(dirname "$$target")" "$(dir $(BIN))"; \
	if [ ! -e "$$target" ]; then \
		tmp="$$target.tmp-$$$$"; cp "$(BINARY)" "$$tmp"; chmod 755 "$$tmp"; mv "$$tmp" "$$target"; \
	fi; \
	ln -sfn "$$target" "$(BIN)"; \
	echo "$(BIN) -> $$target"

skill: build
	node scripts/release/assemble-skill.mjs --dist "$(DIST)"
	node scripts/check-skill-entry.mjs "$(DIST)/skill"

npm-package: build
	node scripts/release/pack.mjs --dist "$(DIST)" --out "$(DIST)/npm" $(PACK_FLAGS)

package: skill npm-package

# Local gate: offline parts, then the networked acceptance and performance gates.
# The installed scanner must not change while it runs.
verify:
	@set -eu; \
	snapshot() { target=$$(readlink "$(BIN)" 2>/dev/null || true); if [ -n "$$target" ] && [ -f "$$target" ]; then hash=$$(shasum -a 256 "$$target" | awk '{print $$1}'); else hash="missing"; fi; printf "%s|%s" "$$target" "$$hash"; }; \
	before=$$(snapshot); \
	status=0; \
	$(MAKE) verify-offline verify-online || status=$$?; \
	after=$$(snapshot); \
	if [ "$$before" != "$$after" ]; then echo "installed scanner changed during make verify: $$before -> $$after" >&2; exit 1; fi; \
	exit "$$status"

# Everything that needs no network. CI runs this on every platform.
verify-offline: build
	node "$(TOOLS)/scripts/verify-build-provenance.js"
	node "$(TOOLS)/scripts/check-registry.js"
	node "$(TOOLS)/scripts/generate-derived-docs.js" --check
	node "$(TOOLS)/scripts/check-skill-thin.js"
	node "$(TOOLS)/scripts/check-profile-routing.js"
	node "$(TOOLS)/scripts/check-evidence-contracts.js"
	node "$(TOOLS)/scripts/check-signal-contracts.js"
	node "$(TOOLS)/scripts/check-effect-capabilities.js"
	node "$(TOOLS)/scripts/check-api-catalog.js"
	node "$(TOOLS)/scripts/check-reference-routing.js"
	node "$(TOOLS)/scripts/typecheck-references.js"
	node "$(TOOLS)/scripts/self-test.js" --exact
	node "$(TOOLS)/scripts/self-test.js" --contract
	"$(BINARY)" . --strict --output gate-json >/tmp/effect-skill-verify-profile.json
	node scripts/release/assemble-skill.mjs --dist "$(DIST)"
	node scripts/check-skill-entry.mjs "$(DIST)/skill"
	node scripts/verify-package.mjs

# Networked gates (npm install, wrangler, benchmark): one CI job runs them.
verify-online:
	EFFECT_SCAN_BIN="$(abspath $(BINARY))" node "$(TOOLS)/scripts/verify-acceptance.js"
	node "$(TOOLS)/scripts/benchmark-scan.js"

verify-acceptance: build
	EFFECT_SCAN_BIN="$(abspath $(BINARY))" node "$(TOOLS)/scripts/verify-acceptance.js"

verify-performance: build
	node "$(TOOLS)/scripts/benchmark-scan.js"

verify-performance-realworld: build
	node "$(TOOLS)/scripts/benchmark-scan.js" --github
