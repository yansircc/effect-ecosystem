import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { compileContractValidators } from "./contract-validation.js";
import { effectVersionFromResolution } from "./resolver.js";
const __dirname = dirname(fileURLToPath(import.meta.url));
const DEFAULT_NOT_PROVEN = [
    { id: "type-level-beyond-lsp", source: "scanner-default" },
    { id: "runtime-behavior", source: "scanner-default" },
    { id: "architecture-boundaries", source: "scanner-default" },
];
const BUNDLE_ARTIFACTS = {
    rawJson: "scan-result.json",
    evidence: "scan-evidence.json",
};
export function writeScanBundle(root, manifest, files, toolchainMeta, scanState, evidenceDir, rawResult) {
    const { evidence, inputHash, fullHash, gateSummary } = buildScanProjection(root, manifest, toolchainMeta, scanState, rawResult, BUNDLE_ARTIFACTS);
    const outDir = resolve(evidenceDir);
    mkdirSync(outDir, { recursive: true });
    const bundle = {
        files: ["scan-result.json", "scan-evidence.json", "gate-summary.json", "input.sha256", "full.sha256"],
        inputHash,
        fullHash,
        complianceHash: gateSummary.complianceHash,
    };
    writeFileSync(resolve(outDir, "scan-result.json"), `${stableJson({ ...rawResult, evidence: bundle })}\n`);
    writeFileSync(resolve(outDir, "scan-evidence.json"), `${stableJson(evidence)}\n`);
    writeFileSync(resolve(outDir, "gate-summary.json"), `${stableJson(gateSummary)}\n`);
    writeFileSync(resolve(outDir, "input.sha256"), `${inputHash}\n`);
    writeFileSync(resolve(outDir, "full.sha256"), `${fullHash}\n`);
    return { bundle, gateSummary };
}
export function buildScanProjection(root, manifest, toolchainMeta, scanState, rawResult, artifacts = null) {
    const { evidence, inputHash, fullHash } = buildScanEvidence(root, toolchainMeta, scanState, rawResult.profile?.requiredReferences ?? []);
    const gateSummary = buildGateSummary(rawResult, evidence, { inputHash, fullHash }, manifest, artifacts);
    const validators = compileContractValidators();
    const evidenceValidation = validators.validateScanEvidence(evidence);
    if (!evidenceValidation.ok)
        throw new Error(evidenceValidation.message);
    const gateValidation = validators.validateGateSummary(gateSummary);
    if (!gateValidation.ok)
        throw new Error(gateValidation.message);
    return { evidence, inputHash, fullHash, gateSummary };
}
function buildScanEvidence(root, toolchainMeta, scanState, requiredReferences) {
    const targetInput = {
        target: scanState.target,
        resolution: scanState.resolution,
        capabilities: scanState.capabilities,
        references: {
            required: requiredReferences,
        },
    };
    const scanner = {
        buildInfo: readBuildInfo(),
    };
    const evidence = {
        schemaVersion: 2,
        ...targetInput,
        toolchain: {
            effectVersion: effectVersionFromResolution(scanState),
            typescriptVersion: toolchainMeta?.tscVersion?.match(/Version\s+([^+\s]+)/)?.[1] ?? null,
            tsgoVersion: toolchainMeta?.tsgoVersion ?? null,
            verified: Boolean(toolchainMeta?.available && effectVersionFromResolution(scanState)),
        },
        compiler: {
            fingerprint: toolchainMeta?.fingerprint ?? null,
            transactionCount: toolchainMeta?.transactionCount ?? 0,
        },
        scanner,
    };
    const inputHash = sha256(stableJson(targetInput));
    const fullHash = sha256(stableJson(evidence));
    return { evidence, inputHash, fullHash };
}
function buildGateSummary(result, evidence, hashes, manifest, artifacts) {
    const block = normalizeFindings(result.findings.filter((finding) => finding.severity === "error"));
    const report = normalizeFindings(result.findings.filter((finding) => finding.severity === "warning"));
    const scannerBuildId = evidence.scanner.buildInfo.buildId;
    const complianceHash = sha256(stableJson({
        scannerBuildId,
        findings: [...block, ...report],
    }));
    const signals = result.signals ?? [];
    const byKind = countBy(signals.map((signal) => signal.kind));
    const packageRollups = signals
        .filter((signal) => signal.kind === "library-exported-effect-package")
        .map((signal) => ({ kind: signal.kind, package: signal.package, facts: signal.facts }))
        .sort((a, b) => a.package.localeCompare(b.package));
    return {
        schemaVersion: 2,
        kind: "effect-mechanical-compliance-gate",
        ok: result.summary.errors === 0,
        exitCode: result.summary.errors > 0 ? 1 : 0,
        summary: {
            findings: result.summary.total,
            errors: result.summary.errors,
            warnings: result.summary.warnings,
            byRule: result.summary.byRule,
        },
        effect: {
            version: result.profile?.effectVersion ?? null,
            support: result.profile?.effectSupport ?? "unresolved",
            activeProfiles: result.profile?.activeProfiles ?? [],
            requiredReferences: result.profile?.requiredReferences ?? [],
        },
        tiers: {
            block,
            report,
            review: {
                signals: {
                    total: signals.length,
                    byKind,
                    blocking: false,
                    packageRollups,
                },
            },
        },
        scanner: {
            buildId: scannerBuildId,
            gitCommit: evidence.scanner.buildInfo.gitCommit,
            dirty: evidence.scanner.buildInfo.dirty,
            inputHash: hashes.inputHash,
            fullHash: hashes.fullHash,
            comparePolicy: {
                compatibleWhen: "scannerBuildId equal",
                onScannerBuildIdChange: "re-baseline",
            },
        },
        complianceHash,
        notProven: [
            ...DEFAULT_NOT_PROVEN,
            ...(manifest?.gate?.notProven ?? []).map((item) => ({
                id: item.id,
                source: "manifest",
                owner: item.owner,
                reason: item.reason,
            })),
        ],
        artifacts,
    };
}
function normalizeFindings(findings) {
    return findings
        .map((finding) => ({
        ruleId: finding.ruleId,
        severity: finding.severity,
        file: finding.file,
        line: finding.line,
        message: finding.message,
        ...(finding.ref ? { ref: finding.ref } : {}),
    }))
        .sort((a, b) => {
        const rule = a.ruleId.localeCompare(b.ruleId);
        if (rule !== 0)
            return rule;
        const file = a.file.localeCompare(b.file);
        if (file !== 0)
            return file;
        return a.line - b.line;
    });
}
function countBy(values) {
    const out = {};
    for (const value of values)
        out[value] = (out[value] ?? 0) + 1;
    return Object.fromEntries(Object.entries(out).sort(([a], [b]) => a.localeCompare(b)));
}
export function stableJson(value) {
    return JSON.stringify(sortJson(value), null, 2);
}
export function sha256(value) {
    return createHash("sha256").update(value).digest("hex");
}
function readBuildInfo() {
    const path = resolve(__dirname, "..", "..", "build-info.json");
    if (!existsSync(path)) {
        return {
            schemaVersion: 1,
            gitCommit: null,
            dirty: null,
            buildId: null,
            sourceRootRelative: null,
        };
    }
    return JSON.parse(readFileSync(path, "utf8"));
}
function sortJson(value) {
    if (Array.isArray(value))
        return value.map(sortJson);
    if (!value || typeof value !== "object")
        return value;
    return Object.fromEntries(Object.entries(value).sort(([a], [b]) => a.localeCompare(b)).map(([key, child]) => [key, sortJson(child)]));
}
