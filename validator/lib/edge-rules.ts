import * as ts from "typescript/unstable/ast"
import { platformConstructors } from "./platform-constructors.js"
import { makeFinding } from "./findings.js"
import { effectRunCallsForFile, verifyExecutionContract } from "./execution-contract.js"
import { compilerProgramForFile } from "./compiler-facts.js"

const NAMESPACE_IMPORT_RE = /import\s+\*\s+as\s+\w+\s+from\s+['"](effect|@effect\/[^'"]+)['"]/

export function scanEdgeRules(root, files, manifest, constructorRule) {
  if (!manifest) return []
  const findings = []
  for (const file of files) {
    const isEdge = file.roles.executableEdge
    const allowsPlatformCtor = file.allowedAdapterRules.includes("EFF402")
    if (!isEdge) {
      for (const run of effectRunCallsForFile(file)) {
        findings.push(makeFinding(root, {
          ruleId: "EFF400",
          file: file.relative,
          line: run.line,
          lineText: run.lineText,
          package: file.package?.path ?? null,
        }))
      }
    }
    if (!allowsPlatformCtor) {
      for (const constructor of platformConstructors(file, constructorRule.scanner.constructors)) {
        if (file.roles.test && constructor.verified && constructorRule.scanner.testFixtures.includes(constructor.name)) continue
        findings.push(makeFinding(root, {
          ruleId: "EFF402",
          file: file.relative,
          line: constructor.line,
          lineText: file.lines[constructor.line - 1]?.text.trim() ?? "",
          package: file.package?.path ?? null,
        }))
      }
    }
    for (const line of file.lines) {
      if (NAMESPACE_IMPORT_RE.test(line.text)) {
        findings.push(makeFinding(root, {
          ruleId: "EFF403",
          file: file.relative,
          line: line.number,
          lineText: line.text.trim(),
          package: file.package?.path ?? null,
        }))
      }
    }
    if (file.roles.source) {
      for (const lineNumber of topLevelDynamicImportLines(file)) {
        const line = file.lines[lineNumber - 1]
        findings.push(makeFinding(root, {
          ruleId: "EFF404",
          file: file.relative,
          line: lineNumber,
          lineText: line?.text.trim() ?? "",
          package: file.package?.path ?? null,
        }))
      }
    }
  }

  const edgesByPath = new Map()
  for (const edge of manifest.executableEdges) {
    const edges = edgesByPath.get(edge.path) ?? []
    edges.push(edge)
    edgesByPath.set(edge.path, edges)
  }
  for (const [path, edges] of edgesByPath) {
    if (edges.length > 1) {
      findings.push(makeFinding(root, {
        ruleId: "EFF401",
        file: path,
        line: 1,
        lineText: "",
        message: `executable edge ${path} must have exactly one execution contract`,
      }))
      continue
    }
    const [edge] = edges
    const file = files.find((item) => item.relative === edge.path)
    if (!file) {
      findings.push(makeFinding(root, {
        ruleId: "EFF401",
        file: edge.path,
        line: 1,
        lineText: "",
        message: `executable edge ${edge.path} does not exist or is not a supported JavaScript/TypeScript source file`,
      }))
      continue
    }
    const result = verifyExecutionContract(file, edge.execution, { root, manifest })
    if (result.ok === false) {
      findings.push(makeFinding(root, {
        ruleId: "EFF401",
        file: edge.path,
        line: result.line,
        lineText: result.lineText,
        message: `executable edge ${edge.path} does not match ${edge.execution.lifecycle} contract: ${result.reason}`,
      }))
    }
  }
  return findings
}

function isDynamicImportCall(node: ts.Node): boolean {
  return ts.isCallExpression(node) && node.expression.kind === ts.SyntaxKind.ImportKeyword
}

function isRequireCall(node: ts.Node): boolean {
  return ts.isCallExpression(node)
    && ts.isIdentifier(node.expression)
    && node.expression.text === "require"
}

function isTopLevelStatement(node: ts.Node): boolean {
  let current: ts.Node | undefined = node.parent
  while (current) {
    if (ts.isFunctionLikeDeclaration(current)) return false
    current = current.parent
  }
  return true
}

/**
 * Returns the 1-based line numbers of forbidden dynamic module loads in a
 * source file: any CommonJS `require()` call, plus module-scope (top-level)
 * dynamic `import()` calls. Function-scoped `import()` is a legitimate lazy
 * load and is not flagged.
 */
function topLevelDynamicImportLines(file: any): number[] {
  const { sourceFile } = compilerProgramForFile(file)
  const lines = new Set<number>()
  visit(sourceFile)
  return [...lines].sort((left, right) => left - right)

  function visit(node: ts.Node): void {
    if (isRequireCall(node)) {
      lines.add(sourceFile.getLineAndCharacterOfPosition(node.getStart(sourceFile)).line + 1)
    } else if (isDynamicImportCall(node) && isTopLevelStatement(node)) {
      lines.add(sourceFile.getLineAndCharacterOfPosition(node.getStart(sourceFile)).line + 1)
    }
    node.forEachChild(visit)
  }
}
