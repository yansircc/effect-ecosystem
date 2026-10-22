import { makeFinding } from "./findings.js";
import { effectRunCallsForFile, verifyExecutionContract } from "./execution-contract.js";
const PLATFORM_CTOR_RE = /\bnew\s+(Response|Request|XMLHttpRequest|WebSocket|EventSource)\s*\(/;
const NAMESPACE_IMPORT_RE = /import\s+\*\s+as\s+\w+\s+from\s+['"](effect|@effect\/[^'"]+)['"]/;
const DYNAMIC_IMPORT_RE = /\brequire\s*\(|(^|\s)import\s*\(/;
export function scanEdgeRules(root, files, manifest) {
    if (!manifest)
        return [];
    const findings = [];
    for (const file of files) {
        const isEdge = file.roles.executableEdge;
        const allowsPlatformCtor = file.allowedAdapterRules.includes("EFF402");
        if (!isEdge) {
            for (const run of effectRunCallsForFile(file)) {
                findings.push(makeFinding(root, {
                    ruleId: "EFF400",
                    file: file.relative,
                    line: run.line,
                    lineText: run.lineText,
                    package: file.package?.path ?? null,
                }));
            }
        }
        for (const line of file.lines) {
            if (!allowsPlatformCtor && PLATFORM_CTOR_RE.test(line.text)) {
                findings.push(makeFinding(root, {
                    ruleId: "EFF402",
                    file: file.relative,
                    line: line.number,
                    lineText: line.text.trim(),
                    package: file.package?.path ?? null,
                }));
            }
            if (NAMESPACE_IMPORT_RE.test(line.text)) {
                findings.push(makeFinding(root, {
                    ruleId: "EFF403",
                    file: file.relative,
                    line: line.number,
                    lineText: line.text.trim(),
                    package: file.package?.path ?? null,
                }));
            }
            if (file.roles.source && DYNAMIC_IMPORT_RE.test(line.text)) {
                findings.push(makeFinding(root, {
                    ruleId: "EFF404",
                    file: file.relative,
                    line: line.number,
                    lineText: line.text.trim(),
                    package: file.package?.path ?? null,
                }));
            }
        }
    }
    const edgesByPath = new Map();
    for (const edge of manifest.executableEdges) {
        const edges = edgesByPath.get(edge.path) ?? [];
        edges.push(edge);
        edgesByPath.set(edge.path, edges);
    }
    for (const [path, edges] of edgesByPath) {
        if (edges.length > 1) {
            findings.push(makeFinding(root, {
                ruleId: "EFF401",
                file: path,
                line: 1,
                lineText: "",
                message: `executable edge ${path} must have exactly one execution contract`,
            }));
            continue;
        }
        const [edge] = edges;
        const file = files.find((item) => item.relative === edge.path);
        if (!file) {
            findings.push(makeFinding(root, {
                ruleId: "EFF401",
                file: edge.path,
                line: 1,
                lineText: "",
                message: `executable edge ${edge.path} does not exist or is not a TypeScript file`,
            }));
            continue;
        }
        const result = verifyExecutionContract(file, edge.execution, { root, manifest });
        if (result.ok === false) {
            findings.push(makeFinding(root, {
                ruleId: "EFF401",
                file: edge.path,
                line: result.line,
                lineText: result.lineText,
                message: `executable edge ${edge.path} does not match ${edge.execution.lifecycle} contract: ${result.reason}`,
            }));
        }
    }
    return findings;
}
