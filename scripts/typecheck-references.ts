#!/usr/bin/env node
// Every ts/typescript/tsx code block in references/**/*.md is compiled against the installed
// packages in one batch. A block that cannot compile (an intentional anti-pattern) must say so
// in its fence: ```ts nocheck. Everything else is a claim about the real API and must hold.
import { mkdtempSync, readdirSync, readFileSync, rmSync, statSync, symlinkSync, writeFileSync, mkdirSync } from "node:fs"
import { tmpdir } from "node:os"
import { dirname, relative, resolve } from "node:path"
import { spawnSync } from "node:child_process"

type Block = { doc: string; startLine: number; code: string; ext: "ts" | "tsx"; id: string }

const root = process.cwd()
const referencesDir = resolve(root, "references")

export function extractBlocks(docPath: string): Block[] {
  const doc = relative(root, docPath)
  const lines = readFileSync(docPath, "utf8").split("\n")
  const blocks: Block[] = []
  for (let index = 0; index < lines.length; index++) {
    const open = lines[index].match(/^```(typescript|ts|tsx)((?:\s+\S+)*)\s*$/)
    if (!open) continue
    const flags = open[2].trim().split(/\s+/).filter(Boolean)
    const body: string[] = []
    let end = index + 1
    while (end < lines.length && !lines[end].startsWith("```")) body.push(lines[end++])
    if (!flags.includes("nocheck")) {
      blocks.push({ doc, startLine: index + 2, code: body.join("\n"), ext: open[1] === "tsx" ? "tsx" : "ts", id: `b${blocks.length}` })
    }
    index = end
  }
  return blocks
}

function markdownFiles(dir: string): string[] {
  return readdirSync(dir).flatMap((entry) => {
    const path = resolve(dir, entry)
    return statSync(path).isDirectory() ? markdownFiles(path) : path.endsWith(".md") ? [path] : []
  })
}

// `--only references/foo.md` limits the run to the given docs (authoring loop).
const only = process.argv.slice(2).flatMap((arg, i, all) => (all[i - 1] === "--only" ? [resolve(root, arg)] : []))

const work = mkdtempSync(resolve(tmpdir(), "effect-references-"))
try {
  const files = new Map<string, Block>()
  for (const docPath of markdownFiles(referencesDir).sort().filter((path) => only.length === 0 || only.includes(path))) {
    for (const block of extractBlocks(docPath)) {
      const path = resolve(work, "src", block.doc.replace(/[^\w]+/g, "_"), `${block.id}.${block.ext}`)
      mkdirSync(dirname(path), { recursive: true })
      // `export {}` makes each block its own module so blocks cannot collide.
      writeFileSync(path, `${block.code}\nexport {}\n`)
      files.set(path, block)
    }
  }
  symlinkSync(resolve(root, "node_modules"), resolve(work, "node_modules"), "dir")
  const failures: string[] = []
  const pending = new Set(files.keys())
  // tsc skips semantic diagnostics while any file has syntax errors, so blocks with syntax errors
  // are reported first and excluded from the semantic pass.
  for (let pass = 0; pass < 2 && pending.size > 0; pass++) {
    writeFileSync(resolve(work, "tsconfig.json"), JSON.stringify({
      compilerOptions: {
        target: "ES2022", module: "NodeNext", moduleResolution: "NodeNext", lib: ["ES2022", "DOM"],
        strict: true, noEmit: true, skipLibCheck: true, jsx: "react-jsx", types: ["node"],
      },
      files: [...pending],
    }))
    const result = spawnSync(process.execPath, [resolve(root, "node_modules/typescript/bin/tsc"), "--noEmit", "--pretty", "false", "-p", resolve(work, "tsconfig.json")], {
      cwd: work, encoding: "utf8", maxBuffer: 50_000_000,
    })
    const syntactic = new Set<string>()
    for (const line of `${result.stdout}\n${result.stderr}`.split("\n")) {
      const match = line.match(/^(.+?)\((\d+),\d+\): error (TS\d+): (.*)$/)
      if (!match) continue
      const path = resolve(work, match[1])
      const block = files.get(path)
      if (!block) { failures.push(line); continue }
      if (pass === 0 && Number(match[3].slice(2)) < 2000) syntactic.add(path)
      if (pass === 0 && syntactic.size > 0 && Number(match[3].slice(2)) >= 2000) continue
      failures.push(`${block.doc}:${block.startLine + Number(match[2]) - 1} ${match[3]}: ${match[4]}`)
    }
    if (syntactic.size === 0) break
    for (const path of syntactic) pending.delete(path)
  }
  if (failures.length > 0) {
    for (const failure of failures) console.error(failure)
    console.error(`reference typecheck failed: ${failures.length} errors in ${new Set(failures.map((line) => line.split(":")[0])).size} docs`)
    process.exit(1)
  }
  console.log(`reference typecheck ok (${files.size} blocks)`)
} finally {
  rmSync(work, { recursive: true, force: true })
}
