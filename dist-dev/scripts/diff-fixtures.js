#!/usr/bin/env node
// Differential harness: runs every fixture through the TypeScript scanner (oracle) and the Rust
// scanner and compares, for the rules the Rust build declares implemented:
//   1. findings `(file, line, ruleId, severity)`;
//   2. the projection layer over those findings, on every output surface:
//      - raw-json (`--profile --timings --evidence`): signals, profile, evidence, summary, warnings,
//        and the five evidence-bundle files byte for byte;
//      - gate-json without evidence (`artifacts: null`) and with evidence, byte for byte;
//      - human output, for fixtures whose oracle findings are all implemented rules.
//   The oracle projects the *Rust* findings (`findingsOverride`), so the projection is compared in
//   isolation from rule coverage; findings are compared by (1).
// Only `timings` is ignored. Build provenance (`scanner.*`, `complianceHash`) matches because the Rust
// build embeds the oracle's `build-info.json`; the harness refuses to run otherwise:
//   EFFECT_SCAN_BUILD_INFO=dist-dev/build-info.json cargo build --release -p effect-scan
//   node dist-dev/scripts/diff-fixtures.js [path/to/effect-scan]
import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, realpathSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { forEachFixtureCase } from "../validator/lib/fixtures.js";
import { stableJson } from "../validator/lib/evidence.js";
import { runScan } from "../validator/lib/scanner.js";
const __dirname = dirname(fileURLToPath(import.meta.url));
const buildDir = resolve(__dirname, "..");
const bin = resolve(process.argv[2] ?? "target/debug/effect-scan");
const BUNDLE_FILES = ["scan-result.json", "scan-evidence.json", "gate-summary.json", "input.sha256", "full.sha256"];
const implemented = new Set(JSON.parse(spawnSync(bin, ["--list-implemented-rules"], { encoding: "utf8" }).stdout));
const builtInfo = JSON.parse(spawnSync(bin, ["--build-info"], { encoding: "utf8" }).stdout);
const oracleInfo = JSON.parse(readFileSync(resolve(buildDir, "build-info.json"), "utf8"));
if (stableJson(builtInfo) !== stableJson(oracleInfo)) {
    console.error("diff-fixtures: the Rust binary embeds a different build-info than the TypeScript build; rebuild it with");
    console.error("  EFFECT_SCAN_BUILD_INFO=dist-dev/build-info.json cargo build --release -p effect-scan");
    process.exit(2);
}
const key = (finding) => `${finding.file}:${finding.line}:${finding.ruleId}:${finding.severity}`;
const evidenceRoot = mkdtempSync(resolve(tmpdir(), "effect-skill-diff-"));
let compared = 0;
let diverged = 0;
let humanCompared = 0;
let counter = 0;
const mismatches = [];
// `unproven-fact` notProven items exist only in the Rust scanner (the oracle has no unprovable-constructor
// verdicts); they do not enter any hash, so the oracle comparison ignores them.
function withoutUnproven(text) {
    const doc = tryParse(text);
    if (!doc || !Array.isArray(doc.notProven))
        return text;
    return `${stableJson({ ...doc, notProven: doc.notProven.filter((item) => item.source !== "unproven-fact") })}\n`;
}
forEachFixtureCase((testCase, inputScanRoot, rulesPath) => {
    // The compiler fingerprint hashes absolute paths: compare both scanners on the canonical root.
    const scanRoot = realpathSync(inputScanRoot);
    const strict = Boolean(testCase.strict);
    const drift = Boolean(testCase.failOnSuppressionDrift);
    const flags = [...(strict ? ["--strict"] : []), ...(drift ? ["--fail-on-suppression-drift"] : [])];
    const caseDir = resolve(evidenceRoot, String(counter++));
    const rustDir = resolve(caseDir, "rust");
    const oracleDir = resolve(caseDir, "oracle");
    mkdirSync(caseDir, { recursive: true });
    const rustRun = (args) => spawnSync(bin, [scanRoot, ...args, ...flags, ...(rulesPath ? ["--rules", rulesPath] : [])], { encoding: "utf8", maxBuffer: 50_000_000 });
    const run = rustRun(["--output", "raw-json", "--profile", "--timings", "--evidence", rustDir]);
    if (run.status !== 0 && run.status !== 1) {
        mismatches.push(`${testCase.name}: rust exited ${run.status}: ${run.stderr.trim().split("\n")[0]}`);
        return;
    }
    const rust = JSON.parse(run.stdout);
    const fail = (message) => mismatches.push(`${testCase.name}: ${message}`);
    // 1. findings. `rustFindings` pins fixtures where the Rust scanner deliberately differs from the
    //    oracle (a documented fix); otherwise the oracle's findings are the expectation.
    const oracle = testCase.rustFindings ? null : runScan(scanRoot, { strict, rulesPath, failOnSuppressionDrift: drift });
    const expected = (oracle ? oracle.findings.filter((f) => implemented.has(f.ruleId)).map(key) : [...testCase.rustFindings]).sort();
    if (testCase.rustFindings)
        diverged++;
    const actual = rust.findings.filter((f) => implemented.has(f.ruleId)).map(key).sort();
    compared++;
    if (JSON.stringify(expected) !== JSON.stringify(actual)) {
        mismatches.push(`${testCase.name}:\n  ts   ${JSON.stringify(expected)}\n  rust ${JSON.stringify(actual)}`);
    }
    // `rustNotProven` pins verdicts the Rust scanner reports as not proven (needs type information).
    if (testCase.rustNotProven) {
        const expectedNotProven = [...testCase.rustNotProven].sort();
        const gate = JSON.parse(readFileSync(resolve(rustDir, "gate-summary.json"), "utf8"));
        // unproven-fact ids are `<ruleId>:<file>:<line>`
        const actualNotProven = gate.notProven
            .filter((item) => item.source === "unproven-fact")
            .map((item) => { const [ruleId, file, line] = String(item.id).split(":"); return `${file}:${line}:${ruleId}`; })
            .sort();
        if (JSON.stringify(expectedNotProven) !== JSON.stringify(actualNotProven)) {
            mismatches.push(`${testCase.name} (notProven):\n  expected ${JSON.stringify(expectedNotProven)}\n  rust     ${JSON.stringify(actualNotProven)}`);
        }
    }
    // 2. projection layer. A fixture with `rustProjection` pins a documented detection fix the oracle lacks
    //    (v4 vocabulary in signals): its own `expect*` signal assertions are checked against the Rust
    //    output, and the Rust surfaces are checked against each other. Every other fixture: the oracle
    //    projects the Rust findings and every surface must match byte for byte.
    const exit = rust.summary.errors > 0 ? 1 : 0;
    if (testCase.rustProjection || testCase.rustNotProven) {
        for (const message of signalExpectationFailures(testCase, rust.signals ?? []))
            fail(message);
        const gateRun = rustRun(["--output", "gate-json", "--evidence", resolve(caseDir, "rust-gate")]);
        if (gateRun.stdout !== readFileSync(resolve(rustDir, "gate-summary.json"), "utf8"))
            fail("gate-json stdout differs from gate-summary.json");
        if (gateRun.status !== exit)
            fail(`exit code ${gateRun.status}, expected ${exit}`);
        return;
    }
    const projected = runScan(scanRoot, {
        strict, rulesPath, failOnSuppressionDrift: drift, profile: true, timings: true, gateSummary: true,
        evidenceDir: oracleDir, findingsOverride: rust.findings,
    });
    const { gateSummary, timings: oracleTimings, compilerDiagnostics: oracleDiagnostics, ...oracleDoc } = projected;
    const { timings: rustTimings, compilerDiagnostics: _rustDiagnostics, ...rustDoc } = rust;
    if (typeof rustTimings?.totalMs !== "number" || typeof rustTimings?.stages?.collectSourceFiles !== "number")
        fail("rust --timings missing totalMs/stages.collectSourceFiles");
    if (typeof oracleTimings?.totalMs !== "number")
        fail("oracle timings missing");
    diffJson(fail, "raw-json", oracleDoc, rustDoc);
    for (const file of BUNDLE_FILES) {
        const want = readFileSync(resolve(oracleDir, file), "utf8");
        const got = file === "gate-summary.json" ? withoutUnproven(readFileSync(resolve(rustDir, file), "utf8")) : readFileSync(resolve(rustDir, file), "utf8");
        if (want === got)
            continue;
        if (file === "scan-result.json" && oracleDiagnostics)
            diffJson(fail, `bundle ${file}`, withoutKey(JSON.parse(want), "compilerDiagnostics"), JSON.parse(got));
        else
            diffJson(fail, `bundle ${file}`, tryParse(want), tryParse(got), want, got);
    }
    const gateNoEvidence = `${stableJson({ ...gateSummary, artifacts: null })}\n`;
    const gateWithEvidence = `${stableJson(gateSummary)}\n`;
    const rustGate = rustRun(["--output", "gate-json"]);
    rustGate.stdout = withoutUnproven(rustGate.stdout);
    if (rustGate.stdout !== gateNoEvidence)
        diffJson(fail, "gate-json (no evidence)", tryParse(gateNoEvidence), tryParse(rustGate.stdout), gateNoEvidence, rustGate.stdout);
    const rustGateEvidence = rustRun(["--output", "gate-json", "--evidence", resolve(caseDir, "rust-gate")]);
    rustGateEvidence.stdout = withoutUnproven(rustGateEvidence.stdout);
    if (rustGateEvidence.stdout !== gateWithEvidence)
        diffJson(fail, "gate-json (evidence)", tryParse(gateWithEvidence), tryParse(rustGateEvidence.stdout), gateWithEvidence, rustGateEvidence.stdout);
    if (rustGate.status !== exit || rustGateEvidence.status !== exit)
        fail(`exit code ${rustGate.status}/${rustGateEvidence.status}, expected ${exit}`);
    // 3. human output, where the oracle's own findings are exactly the Rust ones. EFF900 messages come
    //    from a different JSON-schema engine (documented), so they are not comparable here.
    if (!rulesPath && oracle && oracle.findings.every((f) => implemented.has(f.ruleId) && f.ruleId !== "EFF900")) {
        const cli = spawnSync(process.execPath, [resolve(buildDir, "validator", "scan.js"), scanRoot, "--output", "human", "--evidence", resolve(caseDir, "oracle-human"), ...flags], { encoding: "utf8", maxBuffer: 50_000_000 });
        const rustHuman = rustRun(["--output", "human", "--evidence", resolve(caseDir, "rust-human")]);
        humanCompared++;
        if (cli.stdout !== rustHuman.stdout)
            fail(`human stdout differs\n--- ts\n${cli.stdout}\n--- rust\n${rustHuman.stdout}`);
        const warnLines = (text) => text.split("\n").filter((line) => line.startsWith("[warn]")).join("\n");
        if (warnLines(cli.stderr) !== warnLines(rustHuman.stderr))
            fail("human [warn] lines differ");
        if (cli.status !== rustHuman.status)
            fail(`human exit ${rustHuman.status}, ts ${cli.status}`);
    }
});
for (const mismatch of mismatches)
    console.error(`[diff] ${mismatch}`);
console.log(`diff-fixtures: ${compared} compared (${humanCompared} with human output), ${diverged} pinned to documented divergences, ${mismatches.length} mismatches`);
process.exit(mismatches.length === 0 ? 0 : 1);
/** The signal `expect*` assertions of a fixture (same vocabulary as the TypeScript self-test), on `signals`. */
function signalExpectationFailures(testCase, signals) {
    const failures = [];
    const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);
    if (typeof testCase.expectSignalCount === "number" && signals.length !== testCase.expectSignalCount) {
        failures.push(`expected ${testCase.expectSignalCount} signals, got ${signals.length}`);
    }
    for (const kind of testCase.expectSignals ?? []) {
        if (!signals.some((signal) => signal.kind === kind))
            failures.push(`missing signal ${kind}`);
    }
    for (const expected of testCase.expectSignalFacts ?? []) {
        const signal = signals.find((item) => item.kind === expected.kind && (expected.package === undefined || item.package === expected.package));
        if (!signal) {
            failures.push(`missing signal ${expected.kind}`);
            continue;
        }
        for (const [name, value] of Object.entries(expected.facts ?? {})) {
            if (!same(signal.facts?.[name], value))
                failures.push(`expected ${expected.kind}.${name}=${JSON.stringify(value)}, got ${JSON.stringify(signal.facts?.[name])}`);
        }
    }
    return failures;
}
function tryParse(text) {
    try {
        return JSON.parse(text);
    }
    catch {
        return text;
    }
}
function withoutKey(value, name) {
    const { [name]: _dropped, ...rest } = value;
    return rest;
}
/** Reports the first differing JSON path; with texts given, also reports a byte-level difference. */
function diffJson(report, label, want, got, wantText, gotText) {
    const path = firstDifference(want, got, "$");
    if (path !== null)
        report(`${label}: ${path}`);
    else if (wantText !== undefined && wantText !== gotText)
        report(`${label}: same JSON, different bytes`);
}
function firstDifference(want, got, path) {
    if (JSON.stringify(want) === JSON.stringify(got))
        return null;
    if (Array.isArray(want) && Array.isArray(got)) {
        for (let i = 0; i < Math.max(want.length, got.length); i++) {
            const inner = firstDifference(want[i], got[i], `${path}[${i}]`);
            if (inner)
                return inner;
        }
        return `${path} differs`;
    }
    if (want && got && typeof want === "object" && typeof got === "object" && !Array.isArray(want) && !Array.isArray(got)) {
        for (const name of new Set([...Object.keys(want), ...Object.keys(got)])) {
            const inner = firstDifference(want[name], got[name], `${path}.${name}`);
            if (inner)
                return inner;
        }
        return null; // same members in a different order
    }
    return `${path}: ts ${JSON.stringify(want)?.slice(0, 200)} vs rust ${JSON.stringify(got)?.slice(0, 200)}`;
}
