import * as ts from "typescript/unstable/ast"
import { compilerProgramForFile } from "./compiler-facts.js"

const cache = new Map<string, any>()

export function sourceFactsForFile(file) {
  const { sourceFile } = compilerProgramForFile(file)
  const source = sourceFile.text
  const cached = cache.get(file.absolute)
  if (cached?.source === source) return cached.facts

  const masked = source.split("")
  const syntax: Record<string, { line: number }[]> = { "async-function": [], "async-arrow": [], await: [] }
  const imports = []
  const importStarts = new Set<number>()
  const maskRanges = []

  visit(sourceFile)
  for (const range of commentRanges(sourceFile, source)) maskRanges.push(range)
  for (const range of maskRanges) maskRange(masked, source, range.pos, range.end)

  imports.sort((left, right) => left.start - right.start)
  const facts = {
    syntax,
    codeLines: masked.join("").split(/\r?\n/),
    imports,
    semanticCandidates: semanticCandidates(source, imports),
  }
  cache.set(file.absolute, { source, facts })
  return facts

  function recordSyntax(selector: string, node) {
    syntax[selector].push({ line: sourceFile.getLineAndCharacterOfPosition(node.getStart(sourceFile)).line + 1 })
  }

  function visit(node) {
    const asyncModifier = node.modifiers?.find((modifier) => modifier.kind === ts.SyntaxKind.AsyncKeyword)
    if (asyncModifier) {
      if (ts.isArrowFunction(node)) recordSyntax("async-arrow", asyncModifier)
      else if (ts.isFunctionDeclaration(node) || ts.isFunctionExpression(node) || ts.isMethodDeclaration(node)) recordSyntax("async-function", asyncModifier)
    }
    if (ts.isAwaitExpression(node)) recordSyntax("await", node)
    else if (ts.isForOfStatement(node) && node.awaitModifier) recordSyntax("await", node.awaitModifier)
    else if (ts.isVariableDeclarationList(node) && (node.flags & ts.NodeFlags.AwaitUsing) === ts.NodeFlags.AwaitUsing) recordSyntax("await", node)

    if (isMaskedLiteral(node)) {
      maskRanges.push({ pos: node.getStart(sourceFile), end: node.end })
    }

    if (ts.isImportDeclaration(node) || ts.isExportDeclaration(node)) {
      addImport(node.moduleSpecifier, false)
    } else if (ts.isImportEqualsDeclaration(node)
      && ts.isExternalModuleReference(node.moduleReference)) {
      addImport(node.moduleReference.expression, false)
    } else if (ts.isCallExpression(node)
      && node.expression.kind === ts.SyntaxKind.ImportKeyword) {
      addImport(node.arguments[0], true)
    } else if (ts.isImportTypeNode(node)
      && ts.isLiteralTypeNode(node.argument)) {
      addImport(node.argument.literal, true)
    }

    node.forEachChild(visit)
  }

  function addImport(node, dynamic) {
    if (!node || !ts.isStringLiteral(node)) return
    const start = node.getStart(sourceFile)
    if (importStarts.has(start)) return
    importStarts.add(start)
    imports.push({
      specifier: node.text,
      dynamic,
      start,
      end: node.end,
      line: sourceFile.getLineAndCharacterOfPosition(start).line + 1,
    })
  }
}

function commentRanges(sourceFile, source) {
  const ranges = new Map<string, any>()
  collect(sourceFile)
  return [...ranges.values()].sort((left, right) => left.pos - right.pos)

  function collect(node) {
    add(ts.getLeadingCommentRanges(source, node.pos))
    add(ts.getTrailingCommentRanges(source, node.end))
    node.forEachChild(collect)
  }

  function add(items) {
    for (const item of items ?? []) ranges.set(`${item.pos}:${item.end}`, item)
  }
}

function isMaskedLiteral(node) {
  return ts.isStringLiteral(node)
    || ts.isNoSubstitutionTemplateLiteral(node)
    || ts.isTemplateHead(node)
    || ts.isTemplateMiddle(node)
    || ts.isTemplateTail(node)
}

function maskRange(masked, source, start, end) {
  for (let index = start; index < end; index++) {
    if (source[index] !== "\n" && source[index] !== "\r") masked[index] = " "
  }
}

function semanticCandidates(source, imports) {
  const candidates = []
  if (imports.length > 0 && /\bEffect\b/.test(source)) candidates.push("effect-import")
  if (/\bEffect\.(?:run|runSync|runPromise)/.test(source)) candidates.push("effect-run")
  return candidates
}
