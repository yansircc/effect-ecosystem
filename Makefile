# One build output: $(DIST)/ (default dist/).
#   $(DIST)/platforms/<platform>/effect-scan   the native binary (+ build-info.json it embeds)
#   $(DIST)/tools/                             TypeScript development tools (self-test, checks, oracle); never shipped
#   $(DIST)/skill/                             the assembled skill        (make skill)
# Nothing is installed from here: the generator writes only under $(DIST). `effect-scan` on PATH is the
# launcher sm manages for a consumer's `executablesTarget`, resolving to the published skill's runtime;
# a second installer writing the same path would make that path two-owner (`sm apply` refuses it).
DIST ?= dist
TOOLS := $(DIST)/tools
HOST := $(shell node scripts/release/platforms.mjs host)
BINARY := $(DIST)/platforms/$(HOST)/effect-scan
BUILD_INFO := $(TOOLS)/build-info.json

.PHONY: tools build fixtures skill verify verify-offline verify-online verify-acceptance verify-performance verify-performance-realworld

tools:
	rm -rf "$(TOOLS)"
	node scripts/prepare-effect-tsgo.mjs
	./node_modules/.bin/tsc -p tsconfig.json --outDir "$(TOOLS)"
	node "$(TOOLS)/scripts/write-build-info.js" "$(TOOLS)"
	mkdir -p "$(TOOLS)/validator" "$(TOOLS)/references" "$(TOOLS)/contracts/evidence"
	cp validator/manifest.schema.json "$(TOOLS)/validator/"
	cp -R references/. "$(TOOLS)/references/"
	cp contracts/*.json "$(TOOLS)/contracts/"
	cp contracts/evidence/*.json "$(TOOLS)/contracts/evidence/"

# The binary embeds the build provenance written by `tools` (scanner.buildId, complianceHash).
build: tools
	EFFECT_SCAN_BUILD_INFO="$(abspath $(BUILD_INFO))" cargo build --release --locked -p effect-scan
	mkdir -p "$(dir $(BINARY))"
	cp target/release/effect-scan "$(BINARY)"
	cp "$(BUILD_INFO)" "$(dir $(BINARY))build-info.json"
	node scripts/release/check-binary.mjs "$(BINARY)" "$(BUILD_INFO)"

# Every fixture under fixtures/ (rules/<RULE_ID>/{good,bad}, scenarios/) and the internal suites, through the binary.
fixtures: build
	node "$(TOOLS)/scripts/run-fixtures.js" --bin "$(BINARY)" --exact

skill: build
	node scripts/release/assemble-skill.mjs --dist "$(DIST)"
	node scripts/check-skill-entry.mjs "$(DIST)/skill"

# Local gate: offline parts, then the networked acceptance and performance gates.
verify: verify-offline verify-online

# Everything that needs no network (including the assembled skill's entrypoint run against this repository). CI runs this on every platform.
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
	node "$(TOOLS)/scripts/run-fixtures.js" --bin "$(BINARY)" --exact
	"$(BINARY)" . --strict --output gate-json >/tmp/effect-skill-verify-profile.json
	node scripts/release/assemble-skill.mjs --dist "$(DIST)"
	node scripts/check-skill-entry.mjs "$(DIST)/skill"
	node scripts/verify-skill-runtime.mjs

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
